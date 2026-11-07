// ---- renderlab world radiance cache kernels (M7 G1: sky visibility) ----
// Composed after the built-in prelude, primary_rays.wgsl, gi_cells.wgsl and one ray
// source (gi_rays_hardware.wgsl or gi_rays_software.wgsl); src/gi_cache.rs owns the
// buffers and the per-frame order:
//   gi_probe    covered pixels list the cells they read, deduplicated per tile; each
//               tile then finds its unique cells once, marks them seen, and lists the
//               missing ones in the frame's request table (once per frame);
//   gi_args_misses sizes the claim and resolve dispatches from the listed misses;
//   gi_claim    each miss picks a free slot as the table stands and raises the slot's
//               claim word to its priority (atomicMax, C4);
//   gi_resolve  the highest claim writes the cell and queues it for tracing; claim and
//               resolve repeat in rounds, so collisions retry within the frame;
//   gi_scan     on event frames only: moved casters invalidate the direction slots
//               whose fixed rays they reach (the whole cell when they reach its
//               cube) and re-queue it, the fixed rotation re-queues
//               (slot+frame) mod P == 0, an environment change re-integrates, and
//               partially traced cells (W1 form 2) re-queue;
//   gi_args     sizes the indirect setup/finish and integrate dispatches;
//   gi_setup    one thread per queued cell: the canonical query point when it has
//               none, then one ray item per unknown (or this phase's) direction;
//   gi_args_rays sizes the rays dispatch from the items setup allocated;
//   gi_rays     one thread per ray item: that direction's visibility bit;
//   gi_finish   one thread per queued cell: the bits into its direction slots and the
//               SH over every slot, replacing the value (W1);
//   gi_integrate the SH again from the stored bits, with no rays.
// Every new or invalidated cell is traced in the frame it appears: no ranking decides
// which cells are traced (W2). No reservoirs or resampling anywhere (C1).

struct GiFrame {
    // The traced primaries' shading region [x0, y0, x1, y1).
    region:vec4<u32>,
    // x: moved caster boxes, y: rotation period (0 off), z: scan flags, w: miss capacity.
    counts:vec4<u32>,
    // x: slots, y: miss list offset, z: trace queue offset, w: integrate queue offset.
    offsets:vec4<u32>,
    // x: request table offset, y: its entry mask (two words per entry), z: ray item
    // list offset (GI_RAY_ITEMS words, then the staged visibility bits, one per item).
    requests:vec4<u32>,
    // Swept world bounds of moved placements: lo.xyz then hi.xyz.
    boxes:array<vec4<f32>,64>,
};
const GI_SCAN_CASTER:u32=1u;
const GI_SCAN_ROTATION:u32=2u;
const GI_SCAN_ENVIRONMENT:u32=4u;
const GI_SCAN_PARTIAL:u32=8u;
// Work counters: misses, traces, integrations, then statistics.
const GI_W_MISSES:u32=0u;
const GI_W_TRACES:u32=1u;
const GI_W_INTEGRATES:u32=2u;
const GI_W_MISS_OVERFLOW:u32=3u;
const GI_W_FULL:u32=4u;
const GI_W_ORPHANS:u32=5u;
const GI_W_LOCAL_OVERFLOW:u32=6u;
const GI_W_CREATED:u32=7u;
const GI_W_INVALIDATED:u32=8u;
const GI_W_REFRESHED:u32=9u;
const GI_W_LOST:u32=10u;
const GI_W_RAYS:u32=11u;
// Ray items allocated this frame (may pass the capacity: those cells trace in setup).
const GI_W_ITEMS:u32=12u;
// Ray item capacity, a multiple of 32 (src/gi_cache.rs RAY_ITEMS).
const GI_RAY_ITEMS:u32=1048576u;
const GI_W_CLAIMS:u32=16u;
// A miss: definition (4 words), request-table entry, claimed slot.
const GI_MISS_WORDS:u32=6u;
const GI_UNCLAIMED:u32=0xffffffffu;
const GI_CREATED:u32=0xfffffffeu;
// Prefiltered environment roughness whose lobe is about one direction's solid angle.
const GI_ENV_ROUGHNESS:f32=0.35;

@group(0) @binding(30) var<storage,read_write> gi_table:array<u32>;
@group(0) @binding(31) var<storage,read_write> gi_work:array<atomic<u32>>;
@group(0) @binding(32) var<uniform> gi_frame:GiFrame;
struct GiVec4s { data:array<vec4<f32>> };
struct GiU32s { data:array<u32> };
@group(1) @binding(2) var<storage,read> gi_vertices:binding_array<GiVec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> gi_indices:binding_array<GiU32s,PRIMARY_BATCHES>;

// ---- the shared receiver of one covered pixel ----
struct GiReceiver { world:vec3<f32>, geometric:vec3<f32> };
// The same receiver the traced shading entry builds (program_traced_entry.wgsl):
// receiver_point on the hit triangle in the placement's frame, and its plane normal.
fn gi_receiver(record:vec4<u32>) -> GiReceiver {
    let placement=record.x-1u;
    let batch=primary_placements[placement]&0x1fffffffu;
    let i=record.y*3u;
    let ia=gi_indices[batch].data[i];let ib=gi_indices[batch].data[i+1u];let ic=gi_indices[batch].data[i+2u];
    let a=gi_vertices[batch].data[ia*3u].xyz;let b=gi_vertices[batch].data[ib*3u].xyz;let c=gi_vertices[batch].data[ic*3u].xyz;
    let bary=bitcast<vec2<f32>>(record.zw);
    let w=vec3<f32>(1.0-bary.x-bary.y,bary);
    let world=world_position(receiver_point(a,b,c,w),placement).xyz;
    return GiReceiver(world,primary_geometric(a,b,c,placement));
}

// ---- probe: per-tile unique cells, then one lookup each ----
const GI_LOCAL:u32=256u;
var<workgroup> gi_local_key:array<atomic<u32>,GI_LOCAL>;
var<workgroup> gi_local_priority:array<atomic<u32>,GI_LOCAL>;
var<workgroup> gi_local_def:array<vec4<u32>,GI_LOCAL>;

// Adds one cell to the tile's set; the lowest pixel priority that wants it is kept.
fn gi_local_insert(level:u32, bin:u32, cell:vec3<i32>, priority:u32) {
    let key=gi_key(level,bin,cell);
    var h=key.x%GI_LOCAL;
    for(var i=0u;i<GI_LOCAL;i+=1u) {
        let r=atomicCompareExchangeWeak(&gi_local_key[h],0u,key.x);
        if r.exchanged {
            gi_local_def[h]=vec4<u32>(level|(bin<<8u),bitcast<vec3<u32>>(cell));
            atomicMin(&gi_local_priority[h],priority);
            return;
        }
        if r.old_value==key.x {
            atomicMin(&gi_local_priority[h],priority);
            return;
        }
        // A weak exchange may fail spuriously with the slot still empty: retry it.
        if r.old_value!=0u { h=(h+1u)%GI_LOCAL; }
    }
    atomicAdd(&gi_work[GI_W_LOCAL_OVERFLOW],1u);
}
// The cells a receiver reads at one level and bin, as a descriptor: the four bilinear
// cells of its layer, or the containing cell. Where the point lies within rounding of a
// face that decides the cell, the neighbour is requested too, so a shading module's own
// arithmetic always finds its cell. (Bilinear weights vanish at the faces of the
// tangent axes.) x: level, bin<<8, the near-face flags from bit 11 (bilinear: below and
// above along the normal axis; nearest: below xyz, above xyz); yzw: the base cell.
fn gi_request_desc(world:vec3<f32>, level:u32, bin:u32) -> vec4<u32> {
    let u=gi_lattice(world,gi_edge(level),bin);
    let f=u-floor(u);
    let margin=1e-4+4e-7*max(max(abs(u.x),abs(u.y)),abs(u.z));
    let near_low=f<vec3<f32>(margin);
    let near_high=f>vec3<f32>(1.0-margin);
    var flags=0u;
    var base=vec3<i32>(floor(u));
    if (gi_word(GI_H_FLAGS)&GI_F_TRILINEAR)!=0u {
        let normal=gi_axis_mask(bin)>vec3<f32>(0.5);
        base=gi_corners(u,bin).base;
        flags=select(0u,1u,any(near_low&normal))|select(0u,2u,any(near_high&normal));
    } else {
        flags=select(0u,1u,near_low.x)|select(0u,2u,near_low.y)|select(0u,4u,near_low.z)
            |select(0u,8u,near_high.x)|select(0u,16u,near_high.y)|select(0u,32u,near_high.z);
    }
    return vec4<u32>(level|(bin<<8u)|(flags<<11u),bitcast<vec3<u32>>(base));
}
// Adds a descriptor's cells to the tile's set.
fn gi_request_cells(desc:vec4<u32>, priority:u32) {
    let level=desc.x&0xffu;
    let bin=(desc.x>>8u)&7u;
    let flags=(desc.x>>11u)&63u;
    let base=bitcast<vec3<i32>>(desc.yzw);
    if (gi_word(GI_H_FLAGS)&GI_F_TRILINEAR)!=0u {
        // Layers along the normal axis: the containing one, and a neighbour near a face.
        let axis=vec3<i32>(gi_axis_mask(bin)>vec3<f32>(0.5));
        for(var layer=-1;layer<=1;layer+=1) {
            if (layer==-1 && (flags&1u)==0u) || (layer==1 && (flags&2u)==0u) { continue; }
            for(var k=0u;k<4u;k+=1u) {
                gi_local_insert(level,bin,base+gi_corner_offset(bin,k)+axis*layer,priority);
            }
        }
        return;
    }
    let lo=-vec3<i32>((vec3<u32>(flags)>>vec3<u32>(0u,1u,2u))&vec3<u32>(1u));
    let hi=vec3<i32>((vec3<u32>(flags)>>vec3<u32>(3u,4u,5u))&vec3<u32>(1u));
    for(var x=lo.x;x<=hi.x;x+=1) {
        for(var y=lo.y;y<=hi.y;y+=1) {
            for(var z=lo.z;z<=hi.z;z+=1) {
                gi_local_insert(level,bin,base+vec3<i32>(x,y,z),priority);
            }
        }
    }
}
@compute @workgroup_size(8,8) fn gi_probe(@builtin(global_invocation_id) grid:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    for(var i=lane;i<GI_LOCAL;i+=64u) {
        atomicStore(&gi_local_key[i],0u);
        atomicStore(&gi_local_priority[i],0xffffffffu);
    }
    workgroupBarrier();
    let p=grid.xy+gi_frame.region.xy;
    let size=textureDimensions(primary_hit);
    var record=vec4<u32>(0u);
    if all(p<gi_frame.region.zw) && all(p<size) && textureLoad(primary_tiles,p/PRIMARY_TILE,0).x!=0u {
        record=textureLoad(primary_hit,vec2<i32>(p),0);
    }
    if record.x!=0u {
        let r=gi_receiver(record);
        let n=gi_viewed_normal(r.world,r.geometric);
        // The pixel's priority: lower wins. Unique per pixel; its cells share it.
        let priority=p.y*size.x+p.x;
        // Both levels around a switch: inside the blend band, and within rounding of
        // the switch itself, where the shading module's ceil may go either way.
        let pick=gi_levels(r.world,1e-3);
        // Bins on either side of a near tie, and both sides of a near-grazing view.
        let a=abs(n);
        var bins=1u<<gi_bin(n);
        let eps=1e-4;
        if abs(a.x-a.y)<eps || abs(a.x-a.z)<eps || abs(a.y-a.z)<eps {
            bins|=1u<<gi_bin(n+vec3<f32>(eps*sign(n.x),0.0,0.0));
            bins|=1u<<gi_bin(n+vec3<f32>(0.0,eps*sign(n.y),0.0));
            bins|=1u<<gi_bin(n+vec3<f32>(0.0,0.0,eps*sign(n.z)));
        }
        let eye=vec3<f32>(gi_header_f32(GI_H_EYE),gi_header_f32(GI_H_EYE+1u),gi_header_f32(GI_H_EYE+2u));
        let toward=eye-r.world;
        if abs(dot(r.geometric,toward))<1e-4*length(toward) { bins|=1u<<gi_bin(-n); }
        // Every pixel inserts its own requests. (Skipping pixels whose tile neighbour
        // requests the identical cells is exact but measured only ~5 us faster.)
        for(var b=0u;b<6u;b+=1u) {
            if (bins&(1u<<b))==0u { continue; }
            gi_request_cells(gi_request_desc(r.world,pick.lo,b),priority);
            if pick.hi!=pick.lo { gi_request_cells(gi_request_desc(r.world,pick.hi,b),priority); }
        }
    }
    workgroupBarrier();
    // Each unique cell once: found cells are marked seen, missing ones are listed.
    let capacity=gi_frame.counts.w;
    for(var i=lane;i<GI_LOCAL;i+=64u) {
        if atomicLoad(&gi_local_key[i])==0u { continue; }
        let def=gi_local_def[i];
        let key=gi_key(def.x&0xffu,def.x>>8u,bitcast<vec3<i32>>(def.yzw));
        let slot=gi_find(key);
        if slot!=0xffffffffu {
            // Tiles sharing a cell store the same frame: the race is benign, and only the
            // first store per frame dirties the sector.
            let seen=gi_cold(slot)+GI_C_SEEN;
            let frame=gi_word(GI_H_FRAME);
            if gi_table[seen]!=frame { gi_table[seen]=frame; }
            continue;
        }
        // Tiles share cells: the frame's request table lists each missing cell once and
        // keeps the lowest priority of every tile that wants it (inverted, so the
        // cleared table's zero is no request). Any listing order gives the same set.
        let entry=gi_request_entry(key.x);
        if entry.x==0xffffffffu { atomicAdd(&gi_work[GI_W_MISS_OVERFLOW],1u); continue; }
        atomicMax(&gi_work[gi_frame.requests.x+entry.x*2u+1u],0xffffffffu-atomicLoad(&gi_local_priority[i]));
        if entry.y==0u { continue; }
        let at=atomicAdd(&gi_work[GI_W_MISSES],1u);
        if at>=capacity { atomicAdd(&gi_work[GI_W_MISS_OVERFLOW],1u); continue; }
        let base=gi_frame.offsets.y+at*GI_MISS_WORDS;
        atomicStore(&gi_work[base],def.x);
        atomicStore(&gi_work[base+1u],def.y);
        atomicStore(&gi_work[base+2u],def.z);
        atomicStore(&gi_work[base+3u],def.w);
        atomicStore(&gi_work[base+4u],entry.x);
        atomicStore(&gi_work[base+5u],GI_UNCLAIMED);
    }
}
// The request-table entry of key a (linear probing; zero is empty), and whether this
// call inserted it. (0xffffffff, 0) when the table is full.
fn gi_request_entry(key:u32) -> vec2<u32> {
    let mask=gi_frame.requests.y;
    var h=key&mask;
    for(var j=0u;j<64u;j+=1u) {
        let r=atomicCompareExchangeWeak(&gi_work[gi_frame.requests.x+h*2u],0u,key);
        if r.exchanged { return vec2<u32>(h,1u); }
        if r.old_value==key { return vec2<u32>(h,0u); }
        if r.old_value!=0u { h=(h+1u)&mask; }
    }
    return vec2<u32>(0xffffffffu,0u);
}
// A listed miss's winning priority, already inverted (higher wins).
fn gi_miss_priority(base:u32) -> u32 {
    return atomicLoad(&gi_work[gi_frame.requests.x+atomicLoad(&gi_work[base+4u])*2u+1u]);
}

// ---- claim and resolve: deterministic inserts, in rounds ----
fn gi_miss_key(base:u32) -> vec2<u32> {
    let def=atomicLoad(&gi_work[base]);
    let cell=bitcast<vec3<i32>>(vec3<u32>(atomicLoad(&gi_work[base+1u]),atomicLoad(&gi_work[base+2u]),atomicLoad(&gi_work[base+3u])));
    return gi_key(def&0xffu,def>>8u,cell);
}
fn gi_evictable(slot:u32) -> bool {
    if gi_table[gi_hot(slot)]==0u { return true; }
    return gi_word(GI_H_FRAME)-gi_table[gi_cold(slot)+GI_C_SEEN]>gi_word(GI_H_EVICT);
}
@compute @workgroup_size(64) fn gi_claim(@builtin(global_invocation_id) id:vec3<u32>) {
    let count=min(atomicLoad(&gi_work[GI_W_MISSES]),gi_frame.counts.w);
    if id.x>=count { return; }
    let base=gi_frame.offsets.y+id.x*GI_MISS_WORDS;
    if atomicLoad(&gi_work[base+5u])==GI_CREATED { return; }
    let key=gi_miss_key(base);
    let mask=gi_word(GI_H_MASK);
    let bucket=(key.x&mask)&~(GI_BUCKET-1u);
    for(var i=0u;i<GI_BUCKET;i+=1u) {
        let slot=bucket+((key.y+i)&(GI_BUCKET-1u));
        if gi_evictable(slot) {
            atomicStore(&gi_work[base+5u],slot);
            atomicMax(&gi_work[GI_W_CLAIMS+slot],gi_miss_priority(base));
            return;
        }
    }
    atomicStore(&gi_work[base+5u],GI_UNCLAIMED);
    atomicAdd(&gi_work[GI_W_FULL],1u);
}
@compute @workgroup_size(64) fn gi_resolve(@builtin(global_invocation_id) id:vec3<u32>) {
    let count=min(atomicLoad(&gi_work[GI_W_MISSES]),gi_frame.counts.w);
    if id.x>=count { return; }
    let base=gi_frame.offsets.y+id.x*GI_MISS_WORDS;
    let slot=atomicLoad(&gi_work[base+5u]);
    if slot>=GI_CREATED { return; }
    if atomicLoad(&gi_work[GI_W_CLAIMS+slot])!=gi_miss_priority(base) {
        atomicAdd(&gi_work[GI_W_LOST],1u);
        return;
    }
    // Losers read this clear as a lower claim, so clearing first is safe.
    atomicStore(&gi_work[GI_W_CLAIMS+slot],0u);
    atomicStore(&gi_work[base+5u],GI_CREATED);
    let key=gi_miss_key(base);
    let cold=gi_cold(slot);
    gi_table[gi_hot(slot)]=key.x;
    gi_table[cold+GI_C_KEY_B]=key.y;
    gi_table[cold+GI_C_STATE]=GI_QUEUED;
    gi_table[cold+GI_C_SEEN]=gi_word(GI_H_FRAME);
    gi_table[cold+GI_C_DEF]=atomicLoad(&gi_work[base]);
    for(var i=0u;i<3u;i+=1u) { gi_table[cold+GI_C_CELL+i]=atomicLoad(&gi_work[base+1u+i]); }
    for(var w=0u;w<GI_MAX_WORDS;w+=1u) { gi_table[cold+GI_C_KNOWN+w]=0u; }
    gi_queue_trace(slot);
    atomicAdd(&gi_work[GI_W_CREATED],1u);
}
// Queue appends stay within the slot-sized queues: a slot is queued at most once per
// frame (GI_QUEUED), so neither can overflow.
fn gi_queue_trace(slot:u32) {
    let at=atomicAdd(&gi_work[GI_W_TRACES],1u);
    if at<gi_frame.offsets.x { atomicStore(&gi_work[gi_frame.offsets.z+at],slot); }
}
fn gi_queue_integrate(slot:u32) {
    let at=atomicAdd(&gi_work[GI_W_INTEGRATES],1u);
    if at<gi_frame.offsets.x { atomicStore(&gi_work[gi_frame.offsets.w+at],slot); }
}

// ---- the fixed directions of one cell ----
struct GiCell { level:u32, bin:u32, cell:vec3<i32>, centre:vec3<f32>, edge:f32, key:vec2<u32> };
fn gi_cell(slot:u32) -> GiCell {
    let cold=gi_cold(slot);
    let def=gi_table[cold+GI_C_DEF];
    let cell=bitcast<vec3<i32>>(vec3<u32>(gi_table[cold+GI_C_CELL],gi_table[cold+GI_C_CELL+1u],gi_table[cold+GI_C_CELL+2u]));
    let level=def&0xffu;
    let bin=def>>8u;
    let edge=gi_edge(level);
    // The cube's lattice corner sits a quarter cell against the bin (gi_lattice).
    let shift=gi_axis_mask(bin)*select(0.25,-0.25,(bin&1u)!=0u);
    return GiCell(level,bin,cell,(vec3<f32>(cell)+vec3<f32>(0.5)+shift)*edge,edge,
        vec2<u32>(gi_table[gi_hot(slot)]&GI_KEY_BITS,gi_table[cold+GI_C_KEY_B]));
}
// Branchless orthonormal basis (Duff et al. 2017).
fn gi_basis(n:vec3<f32>) -> mat3x3<f32> {
    let s=select(-1.0,1.0,n.z>=0.0);
    let a=-1.0/(s+n.z);
    let b=n.x*n.y*a;
    return mat3x3<f32>(vec3<f32>(1.0+s*n.x*n.x*a,s*b,-s*n.x),vec3<f32>(b,s+n.y*n.y*a,-n.y),n);
}
// Direction k of GI_DIRECTIONS: equal solid angles of the hemisphere about the
// query normal, optionally rotated about it by an angle the key fixes.
fn gi_direction(frame:mat3x3<f32>, key:vec2<u32>, k:u32) -> vec3<f32> {
    let z=1.0-(f32(k)+0.5)/f32(GI_DIRECTIONS);
    let r=sqrt(max(1.0-z*z,0.0));
    let rotation=select(0.0,f32(key.y>>8u)*(6.283185307179586/16777216.0),(gi_word(GI_H_FLAGS)&GI_F_ROTATE)!=0u);
    let phi=f32(k)*2.399963229728653+rotation;
    return frame*vec3<f32>(r*cos(phi),r*sin(phi),z);
}
fn gi_pack_normal(n:vec3<f32>) -> u32 {
    let p=n.xy/(abs(n.x)+abs(n.y)+abs(n.z));
    let q=select((vec2<f32>(1.0)-abs(p.yx))*select(vec2<f32>(-1.0),vec2<f32>(1.0),p>=vec2<f32>(0.0)),p,n.z>=0.0);
    return pack2x16snorm(q);
}
fn gi_unpack_normal(w:u32) -> vec3<f32> {
    let q=unpack2x16snorm(w);
    var n=vec3<f32>(q,1.0-abs(q.x)-abs(q.y));
    let t=max(-n.z,0.0);
    n=vec3<f32>(n.x+select(t,-t,n.x>=0.0),n.y+select(t,-t,n.y>=0.0),n.z);
    return normalize(n);
}
fn gi_origin_offset(q:vec3<f32>, edge:f32) -> f32 {
    let m=max(max(abs(q.x),abs(q.y)),abs(q.z));
    let ulp=bitcast<f32>(bitcast<u32>(max(m,1e-30))+1u)-max(m,1e-30);
    return max(1e-5,32.0*ulp)+1e-4*edge;
}

// ---- SH of the occluded sky over the fixed directions ----
fn gi_sky(d:vec3<f32>) -> vec3<f32> {
    if lighting.map.w!=0.0 { return environment_radiance(environment_direction(d),GI_ENV_ROUGHNESS); }
    return lighting.environment;
}
// One cell's SH, accumulated in a fixed direction order (deterministic).
var<private> gi_cell_sh:array<vec3<f32>,9>;
fn gi_sh_add(d:vec3<f32>, open:bool) {
    if !open { return; }
    let radiance=gi_sky(d)*(6.283185307179586/f32(GI_DIRECTIONS));
    gi_sh_basis(d);
    for(var i=0u;i<9u;i+=1u) { gi_cell_sh[i]+=radiance*gi_y[i]; }
}
// The sum replaces the slot's SH, stored as halves of one f32 scale: no blend with
// any earlier value (W1).
fn gi_store_sh(slot:u32) {
    var m=0.0;
    for(var i=0u;i<9u;i+=1u) {
        let c=abs(gi_cell_sh[i]);
        m=max(m,max(c.x,max(c.y,c.z)));
    }
    let scale=max(m,1e-30);
    let hot=gi_hot(slot);
    for(var w=0u;w<14u;w+=1u) {
        let a=gi_sh_component(2u*w);
        let b=select(0.0,gi_sh_component(min(2u*w+1u,26u)),2u*w+1u<27u);
        gi_table[hot+2u+w]=pack2x16float(vec2<f32>(a,b)/scale);
    }
    gi_table[hot+1u]=bitcast<u32>(scale);
}
fn gi_sh_component(j:u32) -> f32 {
    let v=gi_cell_sh[j/3u];
    let c=j%3u;
    return select(select(v.z,v.y,c==1u),v.x,c==0u);
}

// ---- trace: canonical query point, fixed directions, replace ----
const GI_PROBES:u32=5u;
// A deterministic probe from the key alone (W2): from a fixed point on the cube's
// face along its bin normal, straight down through the cube. The first hit is the
// query point when its viewed side faces the bin, it lies in the cube, and a ray
// from it along its normal does not leave through the inside of a closed surface.
fn gi_canonical(c:GiCell, j:u32) -> vec4<f32> {
    let nb=gi_bin_normal(c.bin);
    let axis=c.bin>>1u;
    let t1=gi_bin_normal(((axis+1u)%3u)*2u);
    let t2=gi_bin_normal(((axis+2u)%3u)*2u);
    var o=vec2<f32>(0.0);
    if j>0u { o=vec2<f32>(select(-0.25,0.25,(j&1u)!=0u),select(-0.25,0.25,j<3u)); }
    // The segment overhangs both faces by a margin, so a surface lying exactly on a
    // face is found by both cells beside it.
    let margin=0.01*c.edge;
    let start=c.centre+nb*(0.5*c.edge+margin)+(t1*o.x+t2*o.y)*c.edge;
    let down=gi_closest(start,-nb,0.0,c.edge+2.0*margin);
    if !down.hit { return vec4<f32>(0.0); }
    let n=select(-down.normal,down.normal,dot(down.normal,nb)>=0.0);
    if gi_bin(n)!=c.bin { return vec4<f32>(0.0); }
    let q=start-nb*down.t;
    let inside=abs(q-c.centre)<=vec3<f32>(0.5*c.edge+margin);
    if !all(inside) { return vec4<f32>(0.0); }
    let offset=gi_origin_offset(q,c.edge);
    let up=gi_closest(q+n*offset,n,0.0,1e20);
    // A hit seen from the back of its source winding means q is inside a solid.
    if up.hit && dot(up.normal,n)>0.0 { return vec4<f32>(0.0); }
    return vec4<f32>(q,bitcast<f32>(gi_pack_normal(n)));
}
// Directions a queued cell traces this frame, in word w: every unknown one (replace
// mode: new cells, and the directions a moved caster reached), or this frame's phase
// (W1 form 2).
fn gi_wanted(known:u32, w:u32, phases:u32, phase:u32) -> u32 {
    if phases==1u { return ~known; }
    var m=0u;
    for(var b=0u;b<32u;b+=1u) {
        if (w*32u+b)%phases==phase { m|=1u<<b; }
    }
    return m;
}
fn gi_point(cold:u32) -> vec3<f32> {
    return vec3<f32>(bitcast<f32>(gi_table[cold+GI_C_POINT]),bitcast<f32>(gi_table[cold+GI_C_POINT+1u]),bitcast<f32>(gi_table[cold+GI_C_POINT+2u]));
}
// One thread per queued cell: the canonical query point when the cell has none, then
// one ray item per direction to trace (gi_rays), contiguous per cell in direction
// order. Past the item capacity the cell traces its directions here instead: the
// same rays and bits either way, only slower.
@compute @workgroup_size(64) fn gi_setup(@builtin(global_invocation_id) id:vec3<u32>) {
    let count=min(atomicLoad(&gi_work[GI_W_TRACES]),gi_frame.offsets.x);
    if id.x>=count { return; }
    let slot=atomicLoad(&gi_work[gi_frame.offsets.z+id.x]);
    let c=gi_cell(slot);
    let cold=gi_cold(slot);
    var fresh=true;
    for(var w=0u;w<GI_WORDS;w+=1u) { fresh=fresh && gi_table[cold+GI_C_KNOWN+w]==0u; }
    if fresh {
        // The first valid start in fixed order: the key alone decides the point.
        var found=false;
        for(var j=0u;j<GI_PROBES;j+=1u) {
            let r=gi_canonical(c,j);
            if any(r!=vec4<f32>(0.0)) {
                for(var i=0u;i<3u;i+=1u) { gi_table[cold+GI_C_POINT+i]=bitcast<u32>(r[i]); }
                gi_table[cold+GI_C_NORMAL]=bitcast<u32>(r.w);
                found=true;
                break;
            }
        }
        if !found {
            gi_table[gi_hot(slot)]=c.key.x;
            gi_table[cold+GI_C_STATE]=GI_TRACED|GI_ORPHAN;
            atomicAdd(&gi_work[GI_W_ORPHANS],1u);
            return;
        }
        gi_table[cold+GI_C_STATE]=GI_QUEUED;
    }
    let phases=max(gi_word(GI_H_PHASES),1u);
    let phase=gi_word(GI_H_FRAME)%phases;
    var rays=0u;
    for(var w=0u;w<GI_WORDS;w+=1u) {
        let m=gi_wanted(gi_table[cold+GI_C_KNOWN+w],w,phases,phase);
        gi_table[cold+GI_C_PENDING+w]=m;
        rays+=countOneBits(m);
    }
    atomicAdd(&gi_work[GI_W_RAYS],rays);
    // Items past the capacity are never written; the rays pass stops at the capacity.
    let capacity=min(gi_word(GI_H_ITEMS),GI_RAY_ITEMS);
    var base=capacity;
    if capacity!=0u { base=atomicAdd(&gi_work[GI_W_ITEMS],rays); }
    if base<capacity && rays<=capacity-base {
        gi_table[cold+GI_C_BASE]=base;
        var at=gi_frame.requests.z+base;
        for(var w=0u;w<GI_WORDS;w+=1u) {
            let m=gi_table[cold+GI_C_PENDING+w];
            for(var b=0u;b<32u;b+=1u) {
                if (m&(1u<<b))!=0u {
                    atomicStore(&gi_work[at],(slot<<7u)|(w*32u+b));
                    at+=1u;
                }
            }
        }
        return;
    }
    // Past the capacity: trace here, in direction order; nothing pending.
    let n=gi_unpack_normal(gi_table[cold+GI_C_NORMAL]);
    let q=gi_point(cold);
    let frame=gi_basis(n);
    let origin=q+n*gi_origin_offset(q,c.edge);
    for(var w=0u;w<GI_WORDS;w+=1u) {
        let m=gi_table[cold+GI_C_PENDING+w];
        var bits=gi_table[cold+GI_C_BITS+w];
        for(var b=0u;b<32u;b+=1u) {
            let bit=1u<<b;
            if (m&bit)==0u { continue; }
            let visible=gi_visible(origin,gi_direction(frame,c.key,w*32u+b));
            bits=select(bits&~bit,bits|bit,visible);
        }
        gi_table[cold+GI_C_BITS+w]=bits;
        gi_table[cold+GI_C_KNOWN+w]=gi_table[cold+GI_C_KNOWN+w]|m;
        gi_table[cold+GI_C_PENDING+w]=0u;
    }
}
// One thread per ray item: one direction's visibility from its cell's query point,
// kept as the bit at the item's index (atomicOr: the same bits in any order).
@compute @workgroup_size(64) fn gi_rays(@builtin(global_invocation_id) id:vec3<u32>) {
    let count=min(atomicLoad(&gi_work[GI_W_ITEMS]),GI_RAY_ITEMS);
    if id.x>=count { return; }
    let item=atomicLoad(&gi_work[gi_frame.requests.z+id.x]);
    let slot=item>>7u;
    let cold=gi_cold(slot);
    let edge=gi_edge(gi_table[cold+GI_C_DEF]&0xffu);
    let key=vec2<u32>(gi_table[gi_hot(slot)]&GI_KEY_BITS,gi_table[cold+GI_C_KEY_B]);
    let n=gi_unpack_normal(gi_table[cold+GI_C_NORMAL]);
    let q=gi_point(cold);
    let origin=q+n*gi_origin_offset(q,edge);
    if gi_visible(origin,gi_direction(gi_basis(n),key,item&127u)) {
        let staged=gi_frame.requests.z+GI_RAY_ITEMS;
        atomicOr(&gi_work[staged+id.x/32u],1u<<(id.x%32u));
    }
}
// One thread per queued cell: its items' bits into the direction slots, then the SH
// over every slot in fixed direction order, replacing the value (W1). Untraced
// directions (W1 form 2) count as open sky until their phase comes round.
@compute @workgroup_size(64) fn gi_finish(@builtin(global_invocation_id) id:vec3<u32>) {
    let count=min(atomicLoad(&gi_work[GI_W_TRACES]),gi_frame.offsets.x);
    if id.x>=count { return; }
    let slot=atomicLoad(&gi_work[gi_frame.offsets.z+id.x]);
    let cold=gi_cold(slot);
    if (gi_table[cold+GI_C_STATE]&GI_ORPHAN)!=0u { return; }
    let c=gi_cell(slot);
    let frame=gi_basis(gi_unpack_normal(gi_table[cold+GI_C_NORMAL]));
    let staged=gi_frame.requests.z+GI_RAY_ITEMS;
    var at=gi_table[cold+GI_C_BASE];
    for(var i=0u;i<9u;i+=1u) { gi_cell_sh[i]=vec3<f32>(0.0); }
    var all_known=true;
    for(var w=0u;w<GI_WORDS;w+=1u) {
        let pending=gi_table[cold+GI_C_PENDING+w];
        var bits=gi_table[cold+GI_C_BITS+w];
        let known=gi_table[cold+GI_C_KNOWN+w]|pending;
        for(var b=0u;b<32u;b+=1u) {
            let bit=1u<<b;
            if (pending&bit)!=0u {
                let visible=(atomicLoad(&gi_work[staged+at/32u])&(1u<<(at%32u)))!=0u;
                bits=select(bits&~bit,bits|bit,visible);
                at+=1u;
            }
            gi_sh_add(gi_direction(frame,c.key,w*32u+b),(bits&bit)!=0u || (known&bit)==0u);
        }
        all_known=all_known && known==0xffffffffu;
        gi_table[cold+GI_C_BITS+w]=bits;
        gi_table[cold+GI_C_KNOWN+w]=known;
        gi_table[cold+GI_C_PENDING+w]=0u;
    }
    gi_store_sh(slot);
    // The item base depends on allocation order: cleared, so the table is deterministic.
    gi_table[cold+GI_C_BASE]=0u;
    gi_table[cold+GI_C_STATE]=select(GI_TRACED|GI_PARTIAL,GI_TRACED,all_known);
    gi_table[gi_hot(slot)]=c.key.x|GI_VALID;
}
@compute @workgroup_size(64) fn gi_integrate(@builtin(global_invocation_id) id:vec3<u32>) {
    let count=min(atomicLoad(&gi_work[GI_W_INTEGRATES]),gi_frame.offsets.x);
    if id.x>=count { return; }
    let slot=atomicLoad(&gi_work[gi_frame.offsets.w+id.x]);
    let c=gi_cell(slot);
    let cold=gi_cold(slot);
    let frame=gi_basis(gi_unpack_normal(gi_table[cold+GI_C_NORMAL]));
    for(var i=0u;i<9u;i+=1u) { gi_cell_sh[i]=vec3<f32>(0.0); }
    for(var k=0u;k<GI_DIRECTIONS;k+=1u) {
        let bit=1u<<(k%32u);
        let visible=(gi_table[cold+GI_C_BITS+k/32u]&bit)!=0u;
        let traced=(gi_table[cold+GI_C_KNOWN+k/32u]&bit)!=0u;
        gi_sh_add(gi_direction(frame,c.key,k),visible || !traced);
    }
    gi_store_sh(slot);
}

// ---- event scan ----
fn gi_box_hit(o:vec3<f32>, d:vec3<f32>, lo:vec3<f32>, hi:vec3<f32>) -> bool {
    let inv=vec3<f32>(1.0)/d;
    let a=(lo-o)*inv;let b=(hi-o)*inv;
    let near=max(max(min(a.x,b.x),min(a.y,b.y)),min(a.z,b.z));
    let far=min(min(max(a.x,b.x),max(a.y,b.y)),max(a.z,b.z));
    return far>=max(near,0.0);
}
// What moved casters reach in one cell: its cube (the canonical probe: the whole cell
// is re-probed) or some of its fixed rays, followed without bound (only those
// directions' slots are re-traced). Returns whether anything was reached.
fn gi_reach(slot:u32, c:GiCell) -> bool {
    let cold=gi_cold(slot);
    let orphan=(gi_table[cold+GI_C_STATE]&GI_ORPHAN)!=0u;
    let q=vec3<f32>(bitcast<f32>(gi_table[cold+GI_C_POINT]),bitcast<f32>(gi_table[cold+GI_C_POINT+1u]),bitcast<f32>(gi_table[cold+GI_C_POINT+2u]));
    let frame=gi_basis(gi_unpack_normal(gi_table[cold+GI_C_NORMAL]));
    var reached:array<u32,GI_MAX_WORDS>;
    var any_ray=false;
    // The rays start a small offset along the normal (gi_origin_offset); the box test
    // runs from that same origin, and the box grows by a margin several times larger.
    let origin=q+gi_unpack_normal(gi_table[cold+GI_C_NORMAL])*gi_origin_offset(q,c.edge);
    for(var b=0u;b<gi_frame.counts.x;b+=1u) {
        let pad=vec3<f32>(1e-3*c.edge+1e-4);
        let lo=gi_frame.boxes[b*2u].xyz-pad;let hi=gi_frame.boxes[b*2u+1u].xyz+pad;
        // The cube, grown by a quarter edge for the probe's start points.
        if all(c.centre+vec3<f32>(0.75*c.edge)>=lo) && all(c.centre-vec3<f32>(0.75*c.edge)<=hi) {
            for(var w=0u;w<GI_MAX_WORDS;w+=1u) { gi_table[cold+GI_C_KNOWN+w]=0u; }
            return true;
        }
        // An orphan has no rays; only a moved cube can give it a query point.
        if orphan { continue; }
        for(var k=0u;k<GI_DIRECTIONS;k+=1u) {
            if gi_box_hit(origin,gi_direction(frame,c.key,k),lo,hi) {
                reached[k/32u]|=1u<<(k%32u);
                any_ray=true;
            }
        }
    }
    if any_ray {
        for(var w=0u;w<GI_WORDS;w+=1u) {
            gi_table[cold+GI_C_KNOWN+w]=gi_table[cold+GI_C_KNOWN+w]&~reached[w];
        }
    }
    return any_ray;
}
@compute @workgroup_size(64) fn gi_scan(@builtin(global_invocation_id) id:vec3<u32>) {
    let slot=id.x;
    if slot>=gi_frame.offsets.x { return; }
    if gi_table[gi_hot(slot)]==0u || gi_evictable(slot) { return; }
    let cold=gi_cold(slot);
    let state=gi_table[cold+GI_C_STATE];
    if (state&GI_QUEUED)!=0u { return; }
    let flags=gi_frame.counts.z;
    var trace=(state&GI_TRACED)==0u || ((flags&GI_SCAN_PARTIAL)!=0u && (state&GI_PARTIAL)!=0u);
    if !trace && (flags&GI_SCAN_CASTER)!=0u && gi_reach(slot,gi_cell(slot)) {
        atomicAdd(&gi_work[GI_W_INVALIDATED],1u);
        trace=true;
    }
    if !trace && (flags&GI_SCAN_ROTATION)!=0u && gi_frame.counts.y!=0u && (slot+gi_word(GI_H_FRAME))%gi_frame.counts.y==0u {
        // A fixed-rotation refresh re-probes and re-traces the whole cell.
        for(var w=0u;w<GI_MAX_WORDS;w+=1u) { gi_table[cold+GI_C_KNOWN+w]=0u; }
        atomicAdd(&gi_work[GI_W_REFRESHED],1u);
        trace=true;
    }
    if trace {
        gi_table[cold+GI_C_STATE]=gi_table[cold+GI_C_STATE]|GI_QUEUED;
        gi_queue_trace(slot);
        return;
    }
    if (flags&GI_SCAN_ENVIRONMENT)!=0u && (state&GI_ORPHAN)==0u {
        gi_queue_integrate(slot);
    }
}

// ---- indirect arguments, in their own buffer: never bound by the dispatch reading them ----
@group(0) @binding(33) var<storage,read_write> gi_dispatch:array<u32,16>;
// One thread per item, 64 per workgroup (at most 2^21 items: 32768 groups).
fn gi_set_dispatch(at:u32, items:u32) {
    gi_dispatch[at]=(items+63u)/64u;
    gi_dispatch[at+1u]=1u;
    gi_dispatch[at+2u]=1u;
}
// Before setup: queued cells (setup and finish) at 0, integrations at 4.
@compute @workgroup_size(1) fn gi_args() {
    gi_set_dispatch(0u,atomicLoad(&gi_work[GI_W_TRACES]));
    gi_set_dispatch(4u,atomicLoad(&gi_work[GI_W_INTEGRATES]));
}
// After setup, which allocates them: ray items at 8.
@compute @workgroup_size(1) fn gi_args_rays() {
    gi_set_dispatch(8u,min(atomicLoad(&gi_work[GI_W_ITEMS]),GI_RAY_ITEMS));
}
// After the probe: the listed misses (claim and resolve) at 12. The count may pass the
// list's capacity; those threads return.
@compute @workgroup_size(1) fn gi_args_misses() {
    gi_set_dispatch(12u,min(atomicLoad(&gi_work[GI_W_MISSES]),2097152u));
}
