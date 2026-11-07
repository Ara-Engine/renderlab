// ---- renderlab world radiance cache kernels (M7 G1: sky visibility) ----
// Composed after the built-in prelude, primary_rays.wgsl, gi_cells.wgsl and one ray
// source (gi_rays_hardware.wgsl or gi_rays_software.wgsl); src/gi_cache.rs owns the
// buffers and the per-frame order:
//   gi_probe    covered pixels find their cells, mark them seen and list misses,
//               deduplicated per tile (no table writes besides the seen frame);
//   gi_claim    each miss picks a free slot as the probe left the table and raises
//               the slot's claim word to its priority (atomicMax, C4);
//   gi_resolve  the highest claim writes the cell and queues it for tracing;
//   gi_scan     on event frames only: moved casters invalidate and re-queue every cell
//               whose cube or fixed rays they reach, the fixed rotation re-queues
//               (slot+frame) mod P == 0, and an environment change re-integrates;
//   gi_args     sizes the indirect trace and integrate dispatches;
//   gi_trace    one workgroup per queued cell: the canonical query point, then the
//               fixed directions' visibility and the SH, replacing the value (W1);
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
const GI_W_CLAIMS:u32=16u;
const GI_MISS_WORDS:u32=6u;
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

// ---- probe: lookups, seen marks and per-tile unique misses ----
const GI_LOCAL:u32=512u;
var<workgroup> gi_local_key:array<atomic<u32>,GI_LOCAL>;
var<workgroup> gi_local_priority:array<atomic<u32>,GI_LOCAL>;
var<workgroup> gi_local_def:array<vec4<u32>,GI_LOCAL>;

fn gi_request(level:u32, bin:u32, cell:vec3<i32>, priority:u32) {
    let key=gi_key(level,bin,cell);
    let slot=gi_find(key);
    if slot!=0xffffffffu {
        // Every writer stores the same frame, so the race is benign.
        gi_table[gi_slot_base(slot)+GI_S_SEEN]=gi_word(GI_H_FRAME);
        return;
    }
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
// Cells a receiver needs at one level and bin: the containing cell, or the eight
// trilinear corners. Points within rounding of a cell face also request the
// neighbour, so a shading module's own arithmetic always finds its cell.
fn gi_request_cells(world:vec3<f32>, level:u32, bin:u32, priority:u32) {
    let scaled=world/gi_edge(level);
    if (gi_word(GI_H_FLAGS)&GI_F_TRILINEAR)!=0u {
        let base=vec3<i32>(floor(scaled-vec3<f32>(0.5)));
        for(var c=0u;c<8u;c+=1u) {
            gi_request(level,bin,base+vec3<i32>(vec3<u32>(c&1u,(c>>1u)&1u,(c>>2u)&1u)),priority);
        }
        return;
    }
    let base=floor(scaled);
    let f=scaled-base;
    let margin=1e-4+4e-7*max(max(abs(scaled.x),abs(scaled.y)),abs(scaled.z));
    // Whole-vector selects: dynamic component stores are unreliable (naga, NVIDIA).
    let lo=select(vec3<i32>(0),vec3<i32>(-1),f<vec3<f32>(margin));
    let hi=select(vec3<i32>(0),vec3<i32>(1),f>vec3<f32>(1.0-margin));
    for(var x=lo.x;x<=hi.x;x+=1) {
        for(var y=lo.y;y<=hi.y;y+=1) {
            for(var z=lo.z;z<=hi.z;z+=1) {
                gi_request(level,bin,vec3<i32>(base)+vec3<i32>(x,y,z),priority);
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
        // The pixel's priority: lower wins. Unique per pixel; corners share it.
        let priority=p.y*size.x+p.x;
        // Near an integer the shading module's ceil may round either way: both levels.
        let real=gi_level_real(r.world);
        let levels=gi_word(GI_H_LEVELS);
        var level=gi_level(r.world);
        var alternate=level;
        let k=round(real);
        if abs(real-k)<1e-3 {
            level=min(u32(k),levels-1u);
            alternate=min(u32(k)+1u,levels-1u);
        }
        // Bins on either side of a near tie, and both sides of a near-grazing view.
        let a=abs(n);
        let bin=gi_bin(n);
        var bins=1u<<bin;
        let eps=1e-4;
        if abs(a.x-a.y)<eps || abs(a.x-a.z)<eps || abs(a.y-a.z)<eps {
            bins|=1u<<gi_bin(n+vec3<f32>(eps*sign(n.x),0.0,0.0));
            bins|=1u<<gi_bin(n+vec3<f32>(0.0,eps*sign(n.y),0.0));
            bins|=1u<<gi_bin(n+vec3<f32>(0.0,0.0,eps*sign(n.z)));
        }
        let eye=vec3<f32>(gi_header_f32(GI_H_EYE),gi_header_f32(GI_H_EYE+1u),gi_header_f32(GI_H_EYE+2u));
        let toward=eye-r.world;
        if abs(dot(r.geometric,toward))<1e-4*length(toward) { bins|=1u<<gi_bin(-n); }
        for(var b=0u;b<6u;b+=1u) {
            if (bins&(1u<<b))==0u { continue; }
            gi_request_cells(r.world,level,b,priority);
            if alternate!=level { gi_request_cells(r.world,alternate,b,priority); }
        }
    }
    workgroupBarrier();
    let capacity=gi_frame.counts.w;
    for(var i=lane;i<GI_LOCAL;i+=64u) {
        let key=atomicLoad(&gi_local_key[i]);
        if key==0u { continue; }
        let at=atomicAdd(&gi_work[GI_W_MISSES],1u);
        if at>=capacity { atomicAdd(&gi_work[GI_W_MISS_OVERFLOW],1u); continue; }
        let base=gi_frame.offsets.y+at*GI_MISS_WORDS;
        let def=gi_local_def[i];
        atomicStore(&gi_work[base],def.x);
        atomicStore(&gi_work[base+1u],def.y);
        atomicStore(&gi_work[base+2u],def.z);
        atomicStore(&gi_work[base+3u],def.w);
        atomicStore(&gi_work[base+4u],atomicLoad(&gi_local_priority[i]));
        atomicStore(&gi_work[base+5u],0xffffffffu);
    }
}

// ---- claim and resolve: deterministic inserts ----
fn gi_miss_key(base:u32) -> vec2<u32> {
    let def=atomicLoad(&gi_work[base]);
    let cell=bitcast<vec3<i32>>(vec3<u32>(atomicLoad(&gi_work[base+1u]),atomicLoad(&gi_work[base+2u]),atomicLoad(&gi_work[base+3u])));
    return gi_key(def&0xffu,def>>8u,cell);
}
fn gi_evictable(slot:u32) -> bool {
    let s=gi_slot_base(slot);
    if gi_table[s+GI_S_KEY_A]==0u { return true; }
    return gi_word(GI_H_FRAME)-gi_table[s+GI_S_SEEN]>gi_word(GI_H_EVICT);
}
@compute @workgroup_size(64) fn gi_claim(@builtin(global_invocation_id) id:vec3<u32>) {
    let count=min(atomicLoad(&gi_work[GI_W_MISSES]),gi_frame.counts.w);
    if id.x>=count { return; }
    let base=gi_frame.offsets.y+id.x*GI_MISS_WORDS;
    let key=gi_miss_key(base);
    let mask=gi_word(GI_H_MASK);
    let bucket=(key.x&mask)&~(GI_BUCKET-1u);
    for(var i=0u;i<GI_BUCKET;i+=1u) {
        let slot=bucket+((key.y+i)&(GI_BUCKET-1u));
        if gi_evictable(slot) {
            atomicStore(&gi_work[base+5u],slot);
            // Higher claims win: invert the probe's lowest-wins priority.
            atomicMax(&gi_work[GI_W_CLAIMS+slot],0xffffffffu-atomicLoad(&gi_work[base+4u]));
            return;
        }
    }
    atomicAdd(&gi_work[GI_W_FULL],1u);
}
@compute @workgroup_size(64) fn gi_resolve(@builtin(global_invocation_id) id:vec3<u32>) {
    let count=min(atomicLoad(&gi_work[GI_W_MISSES]),gi_frame.counts.w);
    if id.x>=count { return; }
    let base=gi_frame.offsets.y+id.x*GI_MISS_WORDS;
    let slot=atomicLoad(&gi_work[base+5u]);
    if slot==0xffffffffu { return; }
    if atomicLoad(&gi_work[GI_W_CLAIMS+slot])!=0xffffffffu-atomicLoad(&gi_work[base+4u]) { return; }
    // Losers read this clear as a lower claim, so clearing first is safe.
    atomicStore(&gi_work[GI_W_CLAIMS+slot],0u);
    let key=gi_miss_key(base);
    let s=gi_slot_base(slot);
    gi_table[s+GI_S_KEY_A]=key.x;
    gi_table[s+GI_S_KEY_B]=key.y;
    gi_table[s+GI_S_STATE]=GI_QUEUED;
    gi_table[s+GI_S_SEEN]=gi_word(GI_H_FRAME);
    for(var w=0u;w<GI_MAX_WORDS;w+=1u) { gi_table[s+GI_S_KNOWN+w]=0u; }
    for(var i=0u;i<4u;i+=1u) { gi_table[s+GI_S_DEF+i]=atomicLoad(&gi_work[base+i]); }
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
    let s=gi_slot_base(slot);
    let def=gi_table[s+GI_S_DEF];
    let cell=bitcast<vec3<i32>>(vec3<u32>(gi_table[s+GI_S_CELL],gi_table[s+GI_S_CELL+1u],gi_table[s+GI_S_CELL+2u]));
    let level=def&0xffu;
    let edge=gi_edge(level);
    return GiCell(level,def>>8u,cell,(vec3<f32>(cell)+vec3<f32>(0.5))*edge,edge,
        vec2<u32>(gi_table[s+GI_S_KEY_A],gi_table[s+GI_S_KEY_B]));
}
// Branchless orthonormal basis (Duff et al. 2017).
fn gi_basis(n:vec3<f32>) -> mat3x3<f32> {
    let s=select(-1.0,1.0,n.z>=0.0);
    let a=-1.0/(s+n.z);
    let b=n.x*n.y*a;
    return mat3x3<f32>(vec3<f32>(1.0+s*n.x*n.x*a,s*b,-s*n.x),vec3<f32>(b,s+n.y*n.y*a,-n.y),n);
}
// Direction k of GI_DIRECTIONS: equal solid angles of the hemisphere about the
// query normal, rotated about it by an angle the key fixes.
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

// ---- SH of the occluded sky over the fixed directions (one lane per direction) ----
var<workgroup> gi_reduce:array<array<f32,64>,27>;
var<workgroup> gi_bits:array<atomic<u32>,GI_MAX_WORDS>;
var<workgroup> gi_known:array<atomic<u32>,GI_MAX_WORDS>;
// One lane's SH over its directions (k = lane + 64 j), before the workgroup sum.
const GI_LANES:u32=64u;
var<private> gi_lane_sh:array<vec3<f32>,9>;
fn gi_lane_add(d:vec3<f32>, open:bool) {
    if !open { return; }
    let radiance=gi_sky(d)*(6.283185307179586/f32(GI_DIRECTIONS));
    let y=gi_sh_basis(d);
    for(var i=0u;i<9u;i+=1u) { gi_lane_sh[i]+=radiance*y.c[i]; }
}
fn gi_sky(d:vec3<f32>) -> vec3<f32> {
    if lighting.map.w!=0.0 { return environment_radiance(environment_direction(d),GI_ENV_ROUGHNESS); }
    return lighting.environment;
}
// Lanes contribute v*L*(2pi/K)*Y(d) over their directions; a fixed-order tree sums
// them (deterministic), and the sum replaces the slot's SH. No blend with any
// earlier value (W1).
fn gi_store_sh(slot:u32, lane:u32) {
    for(var i=0u;i<9u;i+=1u) {
        gi_reduce[i*3u][lane]=gi_lane_sh[i].x;
        gi_reduce[i*3u+1u][lane]=gi_lane_sh[i].y;
        gi_reduce[i*3u+2u][lane]=gi_lane_sh[i].z;
    }
    workgroupBarrier();
    for(var stride=32u;stride>0u;stride>>=1u) {
        if lane<stride {
            for(var i=0u;i<27u;i+=1u) { gi_reduce[i][lane]+=gi_reduce[i][lane+stride]; }
        }
        workgroupBarrier();
    }
    if lane<27u { gi_table[gi_slot_base(slot)+GI_S_SH+lane]=bitcast<u32>(gi_reduce[lane][0]); }
}

// ---- trace: canonical query point, fixed directions, replace ----
const GI_PROBES:u32=5u;
var<workgroup> gi_probe_point:array<vec4<f32>,GI_PROBES>;
var<workgroup> gi_probe_valid:array<u32,GI_PROBES>;
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
    // face (an axis plane at a multiple of the edge) is found by both cells beside it.
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
// Indirect dispatches are (min(n, GI_ROW), ceil(n / GI_ROW), 1) workgroups.
const GI_ROW:u32=65535u;
var<workgroup> gi_chosen:u32;
var<workgroup> gi_count:u32;
// The queue length as a workgroup-uniform value, so later barriers stay uniform.
fn gi_queued(lane:u32, counter:u32) -> u32 {
    if lane==0u { gi_count=min(atomicLoad(&gi_work[counter]),gi_frame.offsets.x); }
    return workgroupUniformLoad(&gi_count);
}
@compute @workgroup_size(64) fn gi_trace(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let index=group.y*GI_ROW+group.x;
    if index>=gi_queued(lane,GI_W_TRACES) { return; }
    let slot=atomicLoad(&gi_work[gi_frame.offsets.z+index]);
    let c=gi_cell(slot);
    let s=gi_slot_base(slot);
    // A cell with no traced direction (new or invalidated) probes its query point.
    var fresh=true;
    for(var w=0u;w<GI_WORDS;w+=1u) { fresh=fresh && gi_table[s+GI_S_KNOWN+w]==0u; }
    if lane<GI_MAX_WORDS { atomicStore(&gi_bits[lane],0u); atomicStore(&gi_known[lane],0u); }
    if fresh && lane<GI_PROBES {
        let r=gi_canonical(c,lane);
        gi_probe_point[lane]=r;
        gi_probe_valid[lane]=u32(any(r!=vec4<f32>(0.0)));
    }
    workgroupBarrier();
    // The first valid start in fixed order: the key alone decides the point.
    if lane==0u {
        var first=GI_PROBES+1u;
        if fresh {
            first=GI_PROBES;
            for(var j=0u;j<GI_PROBES;j+=1u) { if gi_probe_valid[j]!=0u { first=j; break; } }
        }
        gi_chosen=first;
    }
    let chosen=workgroupUniformLoad(&gi_chosen);
    if chosen==GI_PROBES {
        if lane==0u {
            gi_table[s+GI_S_STATE]=GI_TRACED|GI_ORPHAN;
            atomicAdd(&gi_work[GI_W_ORPHANS],1u);
        }
        return;
    }
    var q:vec3<f32>;
    var packed:u32;
    if chosen<GI_PROBES {
        let point=gi_probe_point[chosen];
        q=point.xyz;
        packed=bitcast<u32>(point.w);
    } else {
        q=vec3<f32>(bitcast<f32>(gi_table[s+GI_S_POINT]),bitcast<f32>(gi_table[s+GI_S_POINT+1u]),bitcast<f32>(gi_table[s+GI_S_POINT+2u]));
        packed=gi_table[s+GI_S_NORMAL];
    }
    let n=gi_unpack_normal(packed);
    let frame=gi_basis(n);
    let origin=q+n*gi_origin_offset(q,c.edge);
    // This update's directions: all of them, or this frame's phase (W1 form 2).
    let phases=max(gi_word(GI_H_PHASES),1u);
    let phase=gi_word(GI_H_FRAME)%phases;
    for(var i=0u;i<9u;i+=1u) { gi_lane_sh[i]=vec3<f32>(0.0); }
    for(var k=lane;k<GI_DIRECTIONS;k+=GI_LANES) {
        let d=gi_direction(frame,c.key,k);
        let word=k/32u;
        let bit=1u<<(k%32u);
        var visible=(gi_table[s+GI_S_BITS+word]&bit)!=0u;
        var traced=(gi_table[s+GI_S_KNOWN+word]&bit)!=0u;
        if phases==1u || k%phases==phase {
            visible=gi_visible(origin,d);
            traced=true;
        }
        if visible { atomicOr(&gi_bits[word],bit); }
        if traced { atomicOr(&gi_known[word],bit); }
        // Untraced directions count as open sky until their phase comes round.
        gi_lane_add(d,visible || !traced);
    }
    gi_store_sh(slot,lane);
    if lane==0u {
        var all_known=true;
        for(var w=0u;w<GI_WORDS;w+=1u) {
            gi_table[s+GI_S_BITS+w]=atomicLoad(&gi_bits[w]);
            let known=atomicLoad(&gi_known[w]);
            gi_table[s+GI_S_KNOWN+w]=known;
            all_known=all_known && known==0xffffffffu;
        }
        gi_table[s+GI_S_POINT]=bitcast<u32>(q.x);
        gi_table[s+GI_S_POINT+1u]=bitcast<u32>(q.y);
        gi_table[s+GI_S_POINT+2u]=bitcast<u32>(q.z);
        gi_table[s+GI_S_NORMAL]=packed;
        gi_table[s+GI_S_STATE]=select(GI_TRACED|GI_PARTIAL,GI_TRACED,all_known);
    }
}
@compute @workgroup_size(64) fn gi_integrate(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let index=group.y*GI_ROW+group.x;
    if index>=gi_queued(lane,GI_W_INTEGRATES) { return; }
    let slot=atomicLoad(&gi_work[gi_frame.offsets.w+index]);
    let c=gi_cell(slot);
    let s=gi_slot_base(slot);
    let frame=gi_basis(gi_unpack_normal(gi_table[s+GI_S_NORMAL]));
    for(var i=0u;i<9u;i+=1u) { gi_lane_sh[i]=vec3<f32>(0.0); }
    for(var k=lane;k<GI_DIRECTIONS;k+=GI_LANES) {
        let bit=1u<<(k%32u);
        let visible=(gi_table[s+GI_S_BITS+k/32u]&bit)!=0u;
        let traced=(gi_table[s+GI_S_KNOWN+k/32u]&bit)!=0u;
        gi_lane_add(gi_direction(frame,c.key,k),visible || !traced);
    }
    gi_store_sh(slot,lane);
}

// ---- event scan ----
fn gi_box_hit(o:vec3<f32>, d:vec3<f32>, lo:vec3<f32>, hi:vec3<f32>) -> bool {
    let inv=vec3<f32>(1.0)/d;
    let a=(lo-o)*inv;let b=(hi-o)*inv;
    let near=max(max(min(a.x,b.x),min(a.y,b.y)),min(a.z,b.z));
    let far=min(min(max(a.x,b.x),max(a.y,b.y)),max(a.z,b.z));
    return far>=max(near,0.0);
}
// A moved caster can change a cell when its swept box meets the cube (the canonical
// probe) or any of the cell's fixed rays, followed without bound.
fn gi_reached(slot:u32, c:GiCell) -> bool {
    let s=gi_slot_base(slot);
    let orphan=(gi_table[s+GI_S_STATE]&GI_ORPHAN)!=0u;
    let q=vec3<f32>(bitcast<f32>(gi_table[s+GI_S_POINT]),bitcast<f32>(gi_table[s+GI_S_POINT+1u]),bitcast<f32>(gi_table[s+GI_S_POINT+2u]));
    let frame=gi_basis(gi_unpack_normal(gi_table[s+GI_S_NORMAL]));
    for(var b=0u;b<gi_frame.counts.x;b+=1u) {
        let pad=vec3<f32>(1e-4*c.edge+1e-5);
        let lo=gi_frame.boxes[b*2u].xyz-pad;let hi=gi_frame.boxes[b*2u+1u].xyz+pad;
        // The cube, grown by a quarter edge for the probe's start points.
        if all(c.centre+vec3<f32>(0.75*c.edge)>=lo) && all(c.centre-vec3<f32>(0.75*c.edge)<=hi) { return true; }
        if orphan { continue; }
        for(var k=0u;k<GI_DIRECTIONS;k+=1u) {
            if gi_box_hit(q,gi_direction(frame,c.key,k),lo,hi) { return true; }
        }
    }
    return false;
}
@compute @workgroup_size(64) fn gi_scan(@builtin(global_invocation_id) id:vec3<u32>) {
    let slot=id.x;
    if slot>=gi_frame.offsets.x { return; }
    let s=gi_slot_base(slot);
    if gi_table[s+GI_S_KEY_A]==0u || gi_evictable(slot) { return; }
    let state=gi_table[s+GI_S_STATE];
    if (state&GI_QUEUED)!=0u { return; }
    let flags=gi_frame.counts.z;
    var trace=(state&GI_TRACED)==0u || ((flags&GI_SCAN_PARTIAL)!=0u && (state&GI_PARTIAL)!=0u);
    if !trace && (flags&GI_SCAN_CASTER)!=0u && gi_reached(slot,gi_cell(slot)) {
        gi_table[s+GI_S_STATE]=0u;
        for(var w=0u;w<GI_MAX_WORDS;w+=1u) { gi_table[s+GI_S_KNOWN+w]=0u; }
        atomicAdd(&gi_work[GI_W_INVALIDATED],1u);
        trace=true;
    }
    if !trace && (flags&GI_SCAN_ROTATION)!=0u && gi_frame.counts.y!=0u && (slot+gi_word(GI_H_FRAME))%gi_frame.counts.y==0u {
        atomicAdd(&gi_work[GI_W_REFRESHED],1u);
        trace=true;
    }
    if trace {
        gi_table[s+GI_S_STATE]=gi_table[s+GI_S_STATE]|GI_QUEUED;
        gi_queue_trace(slot);
        return;
    }
    if (flags&GI_SCAN_ENVIRONMENT)!=0u && (state&GI_ORPHAN)==0u {
        gi_queue_integrate(slot);
    }
}

// ---- indirect arguments, in their own buffer: never bound by the dispatch reading them ----
@group(0) @binding(33) var<storage,read_write> gi_dispatch:array<u32,8>;
fn gi_rows(n:u32, at:u32) {
    gi_dispatch[at]=min(n,GI_ROW);
    gi_dispatch[at+1u]=(n+GI_ROW-1u)/GI_ROW;
    gi_dispatch[at+2u]=1u;
}
@compute @workgroup_size(1) fn gi_args() {
    gi_rows(atomicLoad(&gi_work[GI_W_TRACES]),0u);
    gi_rows(atomicLoad(&gi_work[GI_W_INTEGRATES]),4u);
}
