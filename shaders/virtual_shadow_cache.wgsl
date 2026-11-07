// N8n surface cache (docs/non-rt-n8n.md), `VirtualLookup::Surface`: the exact lookup of
// shaders/virtual_shadow.wgsl with its results kept per receiver surface cell across frames.
// virtual_shadow_gpu.rs appends this file to virtual_shadow.wgsl, so vsc_missed calls that
// module's per-pixel functions as vsm_lookup does.
//
// The sun and the casters do not move when the camera does, so a surface point's sun
// visibility does not either. Each receiver pixel names a cell: its placement, viewed side, and
// a world-space cube around its point whose edge is a power-of-two multiple of the finest map
// texel, at most VSC_CELL pixels of the receiver's footprint. A pixel shows its cell's count
// without walking only when the cell is solid (fully lit or fully blocked) and the pixel that
// computed it found every receiver of its 3x3 neighbourhood with the same count: the cell then
// lies inside a region that agreed. Every other pixel (a cell beside a shadow edge or penumbra,
// or a cell not yet computed) walks exactly at its own receiver point, so the cache never shows
// a lit receiver darker than its own walk would, nor moves an edge by a cell. Directions are
// jittered by the cell, so a pixel's penumbra noise is anchored to the surface.
//
// Per frame (encode_virtual): vsc_probe looks up every covered pixel's cell in a hash table
// (vsc_table), writes shown cells and queues the other pixels in the lookup's scratch
// (vsm_scratch, the hybrid's layout: count, spread, then two words per pixel; walks known to be
// heavy from the top end); vsc_args sizes the next two dispatches; vsc_missed runs the exact walk
// for the queue, heaviest first; vsc_insert stores the results with the consensus verdict, and a
// shown result also fills the cells between its pixel and its neighbours. A frame that changes
// nothing a count depends on runs no pass; one that keeps the camera (only the sun turned or
// casters moved) keeps the counts of the walking pixels nothing reached.
//
// A value stays valid while
//   - the cache epoch is unchanged (hashed into the key: settings, casters' draw list,
//     receiver producer and table resets, or a wrap of the epochs below, bump it);
//   - the sun has not turned since it was computed, or turned fewer than 16 frames ago and this
//     is not the cell's refresh frame (N8e's lag bound: each cell is refreshed once in 16
//     frames while the sun turns, staggered by world-space block);
//   - no caster moved since, or it was checked on every caster-moving frame since: a hit on such
//     a frame is kept (and marked current) only if N8f's conservative reach test (the receiver's
//     march neighbourhood at every level it marches against the moved casters' light-space
//     bounds) clears it. A cell nobody looked at on a caster-moving frame becomes stale.
// Screen variants: VSC_REUSE 0 with VSC_CANONICAL (and VSC_TRIANGLE) is the first design, every
// cell shown and evaluated at one canonical point; it moved shadow edges by a cell (measured).
// Visibility only: the lookup still outputs the counts that user shading programs consume.

// x frame counter, y sun epoch (bumped on every frame the sun direction changes), z cache epoch,
// w caster epoch (bumped on every frame that moved casters). flags bit 0: this frame moved
// casters; bit 1: it keeps the last frame's camera and receivers (only the sun turned or casters
// moved), so a pixel whose cell is current but not shown keeps its last count: until its cell's
// refresh frame, or until the moved casters reach it. buckets: bucket count - 1 (a power of two
// minus one).
struct VscFrame {frame:u32,sun:u32,epoch:u32,casters:u32,flags:u32,buckets:u32,pad0:u32,pad1:u32};
@group(0) @binding(49) var<uniform> vsc:VscFrame;
// Four 8-byte slots per bucket: (fingerprint, value). Fingerprint 0 is an empty slot. Value:
// visible directions (bits 0-3), the march's absolute start level & 15 (4-7), a check nibble
// of the bucket hash (8-11), the caster epoch & 63 (12-17), whether the value may be shown
// (bit 18, VSC_REUSE), the sun epoch & 8191 (19-31).
@group(0) @binding(50) var<storage,read_write> vsc_table:array<vec2<u32>>;
// VSC_STATS screens: probed receivers, hits, hits at the finer level, refreshes due to the sun,
// cells found but not shown (VSC_REUSE).
@group(0) @binding(51) var<storage,read_write> vsc_stats:array<atomic<u32>,8>;
@group(0) @binding(48) var<storage,read_write> vsc_dispatch:array<u32,8>;
// The insert pass reads the frame's finished counts (the probe's and walks' vsm_output).
@group(0) @binding(52) var vsc_counts:texture_2d<u32>;
const VSC_SHOWN:u32=1u<<18u;
// Which cells a pixel may show without walking (docs/non-rt-n8n.md):
//   0: every cell, its value evaluated at the cell's canonical point (VSC_CANONICAL);
//   1: fully lit cells whose pixel's neighbourhood (VSC_CONSENSUS) was lit too;
//   2: solid cells (fully lit or fully blocked) whose pixel's neighbourhood agreed;
//   3: (cost screens only, unsafe) also cells whose pixel walked to fully blocked without the
//      neighbourhood agreeing: the upper bound of proving those pixels' blocks cheaply.
// Any other pixel walks at its own receiver point, so the cache never darkens a lit receiver
// by a cell's extent. A cell once seen beside a differing count is never shown again (its
// pixels keep walking) until a new epoch.
override VSC_REUSE:u32=2u;
// The consensus neighbourhood's radius in pixels (1: 3x3); 0 shows a solid cell regardless.
override VSC_CONSENSUS:u32=1u;
// Walk the canonical point of the pixel's cell: the cube's centre projected onto the triangle,
// kept off its edges by the resolve's 4-epsilon margin (else the pixel's own receiver point).
override VSC_CANONICAL:bool=false;

// Cell edge in receiver pixels (a power of two keeps cell and map levels in step).
override VSC_CELL:f32=1.0;
// Also accept the next finer cell on a miss (hysteresis while the camera recedes; measured
// rare, 0.1% of probes, for a second probe on every miss).
override VSC_FINER:bool=false;
override VSC_STATS:bool=false;
// Frames a turning sun may age a cell (N8e's rolling refresh bound).
const VSC_LAG:u32=16u;
const VSC_SLOTS:u32=4u;

struct VscCell {level:i32,coords:vec3<i32>,size:f32};
struct VscKey {bucket:u32,fingerprint:u32,check:u32,phase:u32};

// The cell of a receiver point at its footprint, `finer` levels finer.
fn vsc_cell(world:vec3<f32>,finer:i32)->VscCell {
    let footprint=length(world-vsm.eye.xyz)*vsm.eye.w;
    let base=vsm.params.y;
    let level=i32(floor(log2(max(footprint*VSC_CELL,1e-12)/base)))-finer;
    let size=base*exp2(f32(level));
    return VscCell(level,vec3<i32>(floor(world/size)),size);
}
// Whether a cell also belongs to one receiver triangle. The shown cells (VSC_REUSE) are
// certified by their pixel's neighbourhood across triangles, so they need not: at the quarry's
// LOD triangles of about a pixel, a sub-pixel camera step moves most pixels onto another
// triangle, whose cells would be new (measured, 1 px cells: 59% shown while moving with the
// triangle in the key, 86% without).
override VSC_TRIANGLE:bool=false;
fn vsc_chain(seed:u32,slot:u32,primitive:u32,side:u32,level:i32,coords:vec3<i32>)->u32 {
    var h=vsm_hash(seed^slot);
    h=vsm_hash(h^select(0u,primitive,VSC_TRIANGLE || VSC_REUSE==0u));
    h=vsm_hash(h^(u32(level+128)|(side<<8u)));
    h=vsm_hash(h^u32(coords.x));
    h=vsm_hash(h^u32(coords.y));
    return vsm_hash(h^u32(coords.z));
}
// The bucket hashes the cell's 4x4x4-cell world block and places the block's 64 cells in 64
// consecutive buckets (2 KiB), so a tile's probes share cache lines; an independent chain over
// the cell is its fingerprint (52 bits name a cell). Refresh phases are per block too, so a
// turning sun's refreshes stay coherent.
fn vsc_key(slot:u32,primitive:u32,side:u32,c:VscCell)->VscKey {
    let block=c.coords>>vec3<u32>(2u);
    let local=vec3<u32>(c.coords&vec3<i32>(3));
    let h=vsc_chain(vsc.epoch*0x9e3779b9u+0x1234567u,slot,primitive,side,c.level,block);
    let f=vsc_chain(vsc.epoch*0x85ebca6bu+0x7654321u,slot,primitive,side,c.level,c.coords);
    let bucket=((h<<6u)|local.x|(local.y<<2u)|(local.z<<4u))&vsc.buckets;
    return VscKey(bucket,max(f,1u),h>>28u,(h>>24u)&(VSC_LAG-1u));
}
// The opaque receiver's viewed side of its plane (the same for every point of the plane).
fn vsc_side(world:vec3<f32>,geometric:vec3<f32>,thin:bool)->u32 {
    return select(u32(dot(geometric,lighting.eye-world)>=0.0),2u,thin);
}

// N8f's reach test for a cached receiver: its march neighbourhood at every level from the
// finer of its recorded and current start levels, at least through the coarser one, against the
// moved casters' light-space bounds. True when a moved caster may change it.
fn vsc_reached(world:vec3<f32>,current:u32,recorded:u32)->bool {
    if vsm.counts.w==0u {return false;}
    let finest=u32(i32(vsm.levels[0].info.y)&15);
    let index=(recorded+16u-finest)&15u;
    if index>=vsm.counts.x || current==VSM_NONE {return true;}
    let start=min(index,current);let through=max(index,current);
    let p=vsm_light(world);
    for(var l=start;l<vsm.counts.x;l+=1u) {
        let r=vec2<f32>(vsm_radius(l,p.z));
        for(var i=0u;i<vsm.counts.w;i+=1u) {
            let m=vsm.rects[i];
            if all(p.xy+r>=m.xy) && all(p.xy-r<=m.zw) {return true;}
        }
        if l>=through && vsm_cap(l)>=vsm.depth.w-p.z {break;}
    }
    return false;
}

// Whether pixel id lies in a moved caster's old or new screen bounds (vsm_reached's first test).
fn vsc_in_moved(id:vec2<u32>)->bool {
    let c=vec2<f32>(id)+vec2<f32>(0.5);
    for(var i=0u;i<vsm.counts.w;i+=1u) {
        let s=vsm.srects[i];
        if all(c>=s.xy) && all(c<=s.zw) {return true;}
    }
    return false;
}
// Whether pixel id has a receiver the lookup shadows (vsm_prepare's early outs).
fn vsc_shadowed(id:vec2<u32>,record:vec4<u32>,world:vec4<f32>,normal:vec4<f32>)->bool {
    let light=lights[0];
    if record.x==0u || normal.w==0.0 || light.kind.z==0u {return false;}
    let depth=dot(shadow_views[light.kind.y].camera_depth,vec4<f32>(world.xyz,1.0));
    for(var i=0u;i<light.kind.z;i+=1u) {
        let coverage=shadow_views[light.kind.y+i].coverage_bias;
        if depth>=coverage.x && depth<=coverage.y {return true;}
    }
    return false;
}

const VSC_MISS:u32=0xffffffffu;
// A current cell that is not shown (VSC_REUSE): its pixels walk. The low three bits are the bit
// length of its last walk's work (ring tests plus walk visits, N8m's measure; at most 7), which
// a shown cell's count field holds instead.
const VSC_UNSHOWN:u32=0xffffffe0u;
// A cell's cached visible count, VSC_UNSHOWN | its work, or VSC_MISS (absent, or due: stale
// for the sun or the casters). On a caster-moving frame a kept hit is marked current (its caster
// epoch rewritten).
fn vsc_find(c:VscCell,slot:u32,primitive:u32,side:u32,world:vec3<f32>,level:u32)->u32 {
    let key=vsc_key(slot,primitive,side,c);
    let base=key.bucket*VSC_SLOTS;
    let moved=(vsc.flags&1u)!=0u;
    for(var s=0u;s<VSC_SLOTS;s+=1u) {
        let e=vsc_table[base+s];
        if e.x!=key.fingerprint || ((e.y>>8u)&15u)!=key.check {continue;}
        let lag=((vsc.sun&8191u)-(e.y>>19u))&8191u;
        if lag!=0u && (lag>=VSC_LAG || ((vsc.frame+key.phase)&(VSC_LAG-1u))==0u) {
            if VSC_STATS {atomicAdd(&vsc_stats[3],1u);}
            return VSC_MISS;
        }
        let shown=(e.y&VSC_SHOWN)!=0u;
        let epoch=(e.y>>12u)&63u;
        let current=vsc.casters&63u;
        var keep=epoch==current;
        // One caster-moving frame behind: still valid where the moved casters cannot reach.
        if !keep && moved && epoch==((current+63u)&63u) && !vsc_reached(world,level,(e.y>>4u)&15u) {
            vsc_table[base+s]=vec2<u32>(e.x,(e.y&~(63u<<12u))|(current<<12u));
            keep=true;
        }
        if !keep {return VSC_MISS;}
        if !shown {
            if VSC_STATS {atomicAdd(&vsc_stats[4],1u);}
            if VSC_REUSE==3u && (e.y&8u)!=0u {return 0u;}
            return VSC_UNSHOWN|(e.y&7u);
        }
        return e.y&15u;
    }
    return VSC_MISS;
}

var<workgroup> vsc_wg_misses:atomic<u32>;
var<workgroup> vsc_wg_base:u32;
var<workgroup> vsc_wg_heavy:atomic<u32>;
var<workgroup> vsc_wg_heavy_base:u32;
// Unshown cells whose last walk did at least 2^(VSC_HEAVY-1) work units walk first: the probe
// queues them from the queue's top end down (their count in the word before the cursor), the
// others from word 0 up, and the walk takes the top end first (N8m: the slowest walks must not
// start last; measured, the queued walk spent 60% of its time in a tail).
override VSC_HEAVY:u32=5u;
fn vsc_capacity()->u32 {return (arrayLength(&vsm_scratch.words)-4u)/2u;}
fn vsc_heavy_count()->u32 {return arrayLength(&vsm_scratch.words)-2u;}
// The queue entry the walk's k-th lane slot takes: the heavy ones (the top end, in order), then
// the others.
fn vsc_entry(k:u32,heavy:u32)->u32 {return select(k-heavy,vsc_capacity()-1u-k,k<heavy);}
var<workgroup> vsc_wg_hits:atomic<u32>;
var<workgroup> vsc_wg_probes:atomic<u32>;
// The probe's workgroup in pixels (multiples of the 8x4 trace tile); the encoder reads them from
// the built constants. Measured: one tile a workgroup spent 18% of the probe on its coverage load.
override VSC_PROBE_X:u32=16u;
override VSC_PROBE_Y:u32=8u;
// Every covered pixel of the frame, VSC_PROBE_X x VSC_PROBE_Y a workgroup.
@compute @workgroup_size(VSC_PROBE_X,VSC_PROBE_Y) fn vsc_probe(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    if lane==0u {atomicStore(&vsc_wg_misses,0u);atomicStore(&vsc_wg_heavy,0u);atomicStore(&vsc_wg_hits,0u);atomicStore(&vsc_wg_probes,0u);}
    workgroupBarrier();
    let size=textureDimensions(vsm_world);
    let id=gid.xy;
    let inside=all(id<size) && textureLoad(vsm_tiles,id/VSM_TRACE_TILE,0).x!=0u;
    var miss=false;
    var heavy=false;
    if inside {
        let p=vec2<i32>(id);
        let record=textureLoad(vsm_hit,p,0);
        let world=textureLoad(vsm_world,p,0);
        let normal=textureLoad(vsm_normal,p,0);
        if !vsc_shadowed(id,record,world,normal) {
            textureStore(vsm_output,p,vec4<u32>(256u));
        } else {
            let slot=record.x-1u;
            let side=vsc_side(world.xyz,normal.xyz,normal.w<0.0);
            let level=textureLoad(vsm_level_in,p,0).x;
            var visible=vsc_find(vsc_cell(world.xyz,0),slot,record.y,side,world.xyz,level);
            var finer=false;
            if visible==VSC_MISS && VSC_FINER {
                visible=vsc_find(vsc_cell(world.xyz,1),slot,record.y,side,world.xyz,level);
                finer=visible<VSC_UNSHOWN;
            }
            if VSC_STATS {
                atomicAdd(&vsc_wg_probes,1u);
                if visible<VSC_UNSHOWN {atomicAdd(&vsc_wg_hits,1u);}
                if finer {atomicAdd(&vsc_stats[2],1u);}
            }
            // A still frame keeps a walking pixel's last count (its cell is current: neither
            // the sun since its refresh nor anything else changed it), unless the pixel lies in
            // a moved caster's old or new screen bounds, where its receiver may have changed
            // (N8f's screen rectangles; measured without them: 533-778 pixels differed from a
            // fresh lookup after 48 caster-moving frames).
            let unshown=visible>=VSC_UNSHOWN && visible!=VSC_MISS;
            let still=(vsc.flags&2u)!=0u && unshown && !vsc_in_moved(id);
            if visible>=VSC_UNSHOWN {miss=!still;} else {vsm_visible(id,visible);}
            heavy=miss && unshown && (visible&7u)>=VSC_HEAVY;
        }
    }
    var at=0u;
    if miss && !heavy {at=atomicAdd(&vsc_wg_misses,1u);}
    if heavy {at=atomicAdd(&vsc_wg_heavy,1u);}
    workgroupBarrier();
    if lane==0u {
        let n=atomicLoad(&vsc_wg_misses);
        if n!=0u {vsc_wg_base=atomicAdd(&vsm_scratch.queue,n);}
        let h=atomicLoad(&vsc_wg_heavy);
        if h!=0u {vsc_wg_heavy_base=atomicAdd(&vsm_scratch.words[vsc_heavy_count()],h);}
        if VSC_STATS {atomicAdd(&vsc_stats[0],atomicLoad(&vsc_wg_probes));atomicAdd(&vsc_stats[1],atomicLoad(&vsc_wg_hits));}
    }
    workgroupBarrier();
    if miss {
        let k=select(vsc_wg_base+at,vsc_capacity()-1u-(vsc_wg_heavy_base+at),heavy);
        atomicStore(&vsm_scratch.words[2u*k],id.x|(id.y<<16u));
        atomicStore(&vsm_scratch.words[2u*k+1u],0u);
    }
}

// The queued walk's lanes per miss (as vmap_queue_args: a small queue spreads each miss over up to
// VSC_SPREAD lanes, up to VSC_LANES lanes in all), then its dispatch and the insert pass's. N8m's
// schedule: VSC_PERSIST workgroups (0: one per 32 lanes) take 32-lane chunks from a cursor, the
// queue buffer's last word (after the queue's), which this pass zeroes.
override VSC_SPREAD:u32=8u;
override VSC_LANES:u32=8192u;
override VSC_PERSIST:u32=4096u;
fn vsc_cursor()->u32 {return arrayLength(&vsm_scratch.words)-1u;}
@compute @workgroup_size(1) fn vsc_args() {
    let heavy=atomicLoad(&vsm_scratch.words[vsc_heavy_count()]);
    let light=atomicLoad(&vsm_scratch.queue);
    let n=light+heavy;
    vsm_scratch.pad0=heavy;
    let spread=clamp(VSC_LANES/max(n,1u),1u,max(VSC_SPREAD,1u));
    vsm_scratch.spread=spread;
    atomicStore(&vsm_scratch.words[vsc_cursor()],0u);
    let groups=(n*spread+31u)/32u;
    vsc_dispatch[0]=select(groups,min(groups,VSC_PERSIST),VSC_PERSIST!=0u);vsc_dispatch[1]=1u;vsc_dispatch[2]=1u;
    vsc_dispatch[4]=(n+63u)/64u;vsc_dispatch[5]=1u;vsc_dispatch[6]=1u;
}

// vsm_prepare at the pixel's receiver point (VSC_CANONICAL: its cell's canonical point), with the
// cell's jitter.
fn vsc_prepare(id:vec2<u32>)->VsmPixel {
    var out:VsmPixel;
    let p=vec2<i32>(id);
    let record=textureLoad(vsm_hit,p,0);
    let normal=textureLoad(vsm_normal,p,0);
    let world=textureLoad(vsm_world,p,0);
    if !vsc_shadowed(id,record,world,normal) {return out;}
    let light=lights[0];
    let depth=dot(shadow_views[light.kind.y].camera_depth,vec4<f32>(world.xyz,1.0));
    var layer=VSM_NONE;
    for(var i=0u;i<light.kind.z;i+=1u) {
        let coverage=shadow_views[light.kind.y+i].coverage_bias;
        if depth>=coverage.x && depth<=coverage.y {layer=light.kind.y+i;break;}
    }
    let view=shadow_views[layer];
    let slot=record.x-1u;
    let batch=primary_placements[slot]&0x1fffffffu;
    let primitive=record.y;
    let thin=normal.w<0.0;
    let side=vsc_side(world.xyz,normal.xyz,thin);
    let cell=vsc_cell(world.xyz,0);
    vsm_own=vsm_tri(batch,primitive);
    // The pixel's surface point and its offset from it along the normal (N1c's LOD lift).
    let bary=bitcast<vec2<f32>>(record.zw);
    let a=vsm_to_world(vsm_own[0],slot);let b=vsm_to_world(vsm_own[1],slot);let c=vsm_to_world(vsm_own[2],slot);
    let on=vsm_to_world(fma(vsm_own[2],vec3<f32>(bary.y),fma(vsm_own[1],vec3<f32>(bary.x),vsm_own[0]*(1.0-bary.x-bary.y))),slot);
    let n=normal.xyz;
    let lift=n*dot(world.xyz-on,n);
    // The cell centre's projection onto the triangle, at least 4 epsilon inside each edge.
    let centre=(vec3<f32>(cell.coords)+vec3<f32>(0.5))*cell.size;
    let e0=b-a;let e1=c-a;let e2=centre-a;
    let d00=dot(e0,e0);let d01=dot(e0,e1);let d11=dot(e1,e1);let d20=dot(e2,e0);let d21=dot(e2,e1);
    let det=d00*d11-d01*d01;
    var w=vec3<f32>(1.0-bary.x-bary.y,bary.x,bary.y);
    if det>0.0 {
        let wb=(d11*d20-d01*d21)/det;let wc=(d00*d21-d01*d20)/det;
        let twice=length(cross(e0,e1));
        let margin=min(vec3<f32>(0.25),4.0*world.w*vec3<f32>(length(e1-e0),length(e1),length(e0))/max(twice,1e-30));
        w=max(vec3<f32>(1.0-wb-wc,wb,wc),margin);
        w/=w.x+w.y+w.z;
    }
    var point=world.xyz;
    if VSC_CANONICAL {point=vsm_to_world(fma(vsm_own[2],vec3<f32>(w.z),fma(vsm_own[1],vec3<f32>(w.y),vsm_own[0]*w.x)),slot)+lift;}
    out.r=VsmReceiver(point,n,world.w,thin,slot,batch,primitive);
    let l=-light.travel_outer.xyz;
    let rows=transpose(view.world_to_clip);let u=normalize(rows[0].xyz);let v=normalize(rows[1].xyz);
    let seed=vsc_chain(0x2545f491u,slot,primitive,side,cell.level,cell.coords);
    let shift=vec2<f32>(f32(seed>>8u),f32(vsm_hash(seed^0xa511e9b3u)>>8u))*(1.0/16777216.0);
    let slope=view.texel.z;
    vsm_active=0xffu>>(8u-vsm.n8e.x);
    vsm_dir_store(0u,vsm_direction(0u,shift,l,u,v,slope));vsm_dir_store(1u,vsm_direction(1u,shift,l,u,v,slope));
    vsm_dir_store(2u,vsm_direction(2u,shift,l,u,v,slope));vsm_dir_store(3u,vsm_direction(3u,shift,l,u,v,slope));
    vsm_dir_store(4u,vsm_direction(4u,shift,l,u,v,slope));vsm_dir_store(5u,vsm_direction(5u,shift,l,u,v,slope));
    vsm_dir_store(6u,vsm_direction(6u,shift,l,u,v,slope));vsm_dir_store(7u,vsm_direction(7u,shift,l,u,v,slope));
    vsm_shared_origin=vsm_origin(out.r,l);
    vsm_prepare_rays(l);
    out.valid=true;
    out.level=textureLoad(vsm_level_in,p,0).x;
    out.l=l;
    return out;
}

// The exact walk for queue lane g (each queued pixel on `spread` lanes). Word 2k+1 gets bit 31,
// the visible count, the start level << 4 and the work's bit length << 8 for vsc_insert.
fn vsc_walk(g:u32) {
    let spread=max(vsm_scratch.spread,1u);
    if g%spread!=0u {return;}
    let heavy=vsm_scratch.pad0;
    let slot_k=g/spread;
    if slot_k>=atomicLoad(&vsm_scratch.queue)+heavy {return;}
    let k=vsc_entry(slot_k,heavy);
    let word=atomicLoad(&vsm_scratch.words[2u*k]);
    let id=vec2<u32>(word&0xffffu,word>>16u);
    let px=vsc_prepare(id);
    if !px.valid {textureStore(vsm_output,vec2<i32>(id),vec4<u32>(256u));return;}
    let r=px.r;
    vsm_visits=0u;
    let ring=vsm_ring(r,(vsm.misc.w&1u)!=0u && !vsm_beyond_horizon(r,px.l));
    var blocked=ring.blocked;
    if VSM_FAR && px.level!=VSM_NONE && blocked!=0xffu {
        let open=vsm_march(r,ring.radius,px.level,0xffu&~blocked);
        blocked=0xffu&~open;
    }
    let visible=countOneBits(vsm_active&~blocked);
    vsm_visible(id,visible);
    let start=u32((i32(vsm.levels[0].info.y)+i32(min(px.level,15u)))&15);
    let work=min(15u,32u-countLeadingZeros(vsm_ring_tests+vsm_visits));
    atomicStore(&vsm_scratch.words[2u*k+1u],0x80000000u|visible|(start<<4u)|(work<<8u));
}
var<workgroup> vsc_wg_chunk:u32;
// The queued pixels, 32 lanes a workgroup: one chunk each (VSC_PERSIST 0), or chunks from the
// cursor until the queue's lanes are taken. A chunk is claimed only after the last one is done.
@compute @workgroup_size(32) fn vsc_missed(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    vsm_lane=li;
    if VSC_PERSIST==0u {vsc_walk(gid.x);return;}
    let lanes=(atomicLoad(&vsm_scratch.queue)+vsm_scratch.pad0)*max(vsm_scratch.spread,1u);
    loop {
        if li==0u {vsc_wg_chunk=atomicAdd(&vsm_scratch.words[vsc_cursor()],1u);}
        let t=workgroupUniformLoad(&vsc_wg_chunk);
        if t*32u>=lanes {break;}
        vsc_walk(t*32u+li);
    }
}

// Whether every receiver within VSC_CONSENSUS pixels of p finished this frame with count c.
fn vsc_agree(p:vec2<i32>,c:u32)->bool {
    let size=vec2<i32>(textureDimensions(vsc_counts));
    let r=i32(VSC_CONSENSUS);
    for(var y=-r;y<=r;y+=1) {
        for(var x=-r;x<=r;x+=1) {
            let q=p+vec2<i32>(x,y);
            if any(q<vec2<i32>(0)) || any(q>=size) {continue;}
            if textureLoad(vsm_hit,q,0).x==0u {continue;}
            if textureLoad(vsc_counts,q,0).x!=c {return false;}
        }
    }
    return true;
}
// Stores a cell: its slot if present, else an empty or stale slot, else (unless `fill`) one
// chosen by the fingerprint. A current cell that was not shown stays so. `fill` stores only a
// cell that is absent and never evicts a current one.
fn vsc_store(key:VscKey,stored:u32,fill:bool) {
    let base=key.bucket*VSC_SLOTS;
    var value=stored;
    var chosen=VSC_SLOTS;
    var stale=VSC_SLOTS;
    for(var s=0u;s<VSC_SLOTS;s+=1u) {
        let e=vsc_table[base+s];
        let lag=((vsc.sun&8191u)-(e.y>>19u))&8191u;
        let current=e.x!=0u && ((e.y>>12u)&63u)==(vsc.casters&63u) && lag==0u;
        if e.x==key.fingerprint {
            if fill && current {return;}
            chosen=s;
            if current && (e.y&VSC_SHOWN)==0u {value&=~VSC_SHOWN;}
            break;
        }
        if stale==VSC_SLOTS && (e.x==0u || !current) {stale=s;}
    }
    if chosen==VSC_SLOTS {
        if fill && stale==VSC_SLOTS {return;}
        chosen=select(stale,(key.fingerprint>>8u)&(VSC_SLOTS-1u),stale==VSC_SLOTS);
    }
    vsc_table[base+chosen]=vec2<u32>(key.fingerprint,value);
}
// A shown cell also fills the cells of the midpoints to its pixel's 3x3 neighbours on the same
// placement and side (and triangle, with VSC_TRIANGLE): those lie inside the neighbourhood that
// agreed, where the next frames' receiver points land (the cache's hit rate under motion is
// otherwise bounded by where earlier points fell).
override VSC_SPLAT:bool=true;
// Stores the queued results (vsc_store), and with VSC_SPLAT a shown one's neighbourhood.
@compute @workgroup_size(64) fn vsc_insert(@builtin(global_invocation_id) gid:vec3<u32>) {
    let heavy=vsm_scratch.pad0;
    if gid.x>=atomicLoad(&vsm_scratch.queue)+heavy {return;}
    let k=vsc_entry(gid.x,heavy);
    let result=atomicLoad(&vsm_scratch.words[2u*k+1u]);
    if (result&0x80000000u)==0u {return;}
    let word=atomicLoad(&vsm_scratch.words[2u*k]);
    let p=vec2<i32>(vec2<u32>(word&0xffffu,word>>16u));
    let record=textureLoad(vsm_hit,p,0);
    let world=textureLoad(vsm_world,p,0);
    let normal=textureLoad(vsm_normal,p,0);
    let side=vsc_side(world.xyz,normal.xyz,normal.w<0.0);
    let slot=record.x-1u;
    let key=vsc_key(slot,record.y,side,vsc_cell(world.xyz,0));
    let visible=result&15u;
    let kd=vsm.n8e.x;
    var shown=VSC_REUSE==0u;
    if VSC_REUSE!=0u && (visible==kd || (VSC_REUSE==2u && visible==0u)) {
        shown=VSC_CONSENSUS==0u || vsc_agree(p,(256u*visible+kd/2u)/kd);
    }
    let epochs=((vsc.casters&63u)<<12u)|((vsc.sun&8191u)<<19u);
    // An unshown cell keeps its walk's work in place of the count (the walk order's estimate).
    // Unshown: the work's bit length (at most 7), and bit 3 when the walk found every direction
    // blocked (VSC_REUSE 3's screen).
    let field=select(min((result>>8u)&15u,7u)|select(0u,8u,visible==0u),visible,shown);
    vsc_store(key,field|(result&0xf0u)|(key.check<<8u)|epochs|select(0u,VSC_SHOWN,shown),false);
    if !VSC_SPLAT || !shown || VSC_REUSE==0u {return;}
    let size=vec2<i32>(textureDimensions(vsm_world));
    for(var n=0u;n<9u;n+=1u) {
        if n==4u {continue;}
        let q=p+vec2<i32>(i32(n%3u)-1,i32(n/3u)-1);
        if any(q<vec2<i32>(0)) || any(q>=size) {continue;}
        let other=textureLoad(vsm_hit,q,0);
        if other.x!=record.x || (VSC_TRIANGLE && other.y!=record.y) {continue;}
        let wq=textureLoad(vsm_world,q,0);
        let nq=textureLoad(vsm_normal,q,0);
        if vsc_side(wq.xyz,nq.xyz,nq.w<0.0)!=side {continue;}
        let mid=(world.xyz+wq.xyz)*0.5;
        let fill=vsc_key(slot,record.y,side,vsc_cell(mid,0));
        vsc_store(fill,(result&0xffu)|(fill.check<<8u)|epochs|VSC_SHOWN,true);
    }
}
