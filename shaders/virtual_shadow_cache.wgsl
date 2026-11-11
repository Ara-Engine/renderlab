// N8n/N8o surface cache (docs/non-rt-n8n.md, docs/non-rt-n8o.md), `VirtualLookup::Surface`: the
// exact lookup of shaders/virtual_shadow.wgsl with its results kept per receiver surface cell
// across frames. virtual_shadow_gpu.rs appends this file to virtual_shadow.wgsl, so vsc_missed
// runs that module's vsm_prepare, vsm_ring and vsm_march: a walking pixel's count is Exact's.
//
// The sun and the casters do not move when the camera does, so a surface point's sun
// visibility does not either. Each receiver pixel names a cell: its placement, viewed side, and
// a world-space cube around its point whose edge is a power-of-two multiple of the finest map
// texel, at most VSC_CELL pixels of the receiver's footprint. A pixel shows its cell's count
// without walking only when the cell is solid (fully lit or fully blocked) and every pixel that
// computed it found every receiver of its 3x3 neighbourhood with the same count: the cell then
// lies inside a region that agreed. Every other pixel (a cell beside a shadow edge or penumbra,
// or a cell not yet computed) walks exactly at its own receiver point, with Exact's directions,
// so the cache never shows a lit receiver darker than its own walk would, nor moves an edge by a
// cell.
//
// Per frame (encode_virtual): vsc_probe looks up every covered pixel's cell in a hash table
// (vsc_table), writes shown cells and queues the other pixels in the lookup's scratch
// (vsm_scratch, the hybrid's layout: count, spread, then two words per pixel) with its cell's
// work estimate; vsc_args sizes the next dispatches; vsc_sort orders the queue heaviest first;
// vsc_missed runs the exact walk for it; vsc_claim and vsc_resolve store the results.
// A frame that changes nothing a count depends on runs no pass; one that keeps the camera (only
// the sun turned or casters moved) keeps the counts of the walking pixels nothing reached.
//
// N8o: every write is order-independent, so a sequence of frames gives the same table and
// counts in any run (docs/non-rt-n8o.md, measured):
//   - inserts take two passes. vsc_claim picks each result's slot from the table as the frame
//     found it and raises the slot's two claim words to its priority (vsc_priority: verdict,
//     margin, pixel) and its complement; vsc_resolve lets the highest claim write the slot. A
//     cell is shown only when every claim on its slot carried the same solid verdict, so all
//     pixels that computed it agreed;
//   - the probe's caster test (vsc_reached) depends on the cell alone, so every pixel of a cell
//     makes the same decision and writes the same value;
//   - a flush clears the table, so no earlier sequence's cells remain.
//
// A value stays valid while
//   - the table was not cleared (settings, casters' draw list, receiver producer, a map rebase,
//     a new height range or level ladder, or a wrap of the epochs below);
//   - the sun has not turned since it was computed, or turned fewer than 16 frames ago and this
//     is not the cell's refresh frame (N8e's lag bound: each cell is refreshed once in 16
//     frames while the sun turns, staggered by world-space block);
//   - no caster moved since, or it was checked on every caster-moving frame since: a hit on such
//     a frame is kept (and marked current) only if N8f's conservative reach test, widened to the
//     cell's cube, clears it, and stays shown only if the test widened to the neighbourhood that
//     certified it (its margin) clears it too. The walk at any point of those regions, and so the
//     cell's count and its verdict, cannot have changed (N8o). A cell nobody looked at on a
//     caster-moving frame becomes stale.
// Visibility only: the lookup still outputs the counts that user shading programs consume.

// x frame counter, y sun epoch (bumped on every frame the sun direction changes), z caster epoch
// (bumped on every frame that moved casters). w flags: bit 0, this frame moved casters; bit 1,
// it keeps the last frame's camera and receivers (only the sun turned or casters moved), so a
// pixel whose cell is current but not shown keeps its last count: until its cell's refresh
// frame, or until the moved casters reach it. buckets: bucket count - 1 (a power of two minus
// one).
struct VscFrame {frame:u32,sun:u32,casters:u32,flags:u32,buckets:u32,pad0:u32,pad1:u32,pad2:u32};
@group(0) @binding(49) var<uniform> vsc:VscFrame;
// Four 8-byte slots per bucket: (fingerprint, value). Fingerprint 0 is an empty slot. Value:
//   bits 0-3: a shown cell's solid count (bit 0: fully lit, else fully blocked) and its margin
//     exponent (bits 1-3, vsc_margin);
//   bits 4-7: the march's absolute start level & 15;
//   bits 8-11: a check nibble of the bucket hash;
//   bits 12-17: the caster epoch & 63;
//   bit 18: shown;
//   bits 19-21: its last walk's work as a bit length (at most 7; N8m's measure, the walk order's
//     estimate when the cell must walk again);
//   bits 22-31: the sun epoch & 1023.
@group(0) @binding(50) var<storage,read_write> vsc_table:array<vec2<u32>>;
// VSC_STATS screens: probed receivers, hits, cells the caster test retired, refreshes due to the
// sun, cells found but not shown, certified walks whose neighbourhood exceeded the largest
// margin, fill claims.
@group(0) @binding(51) var<storage,read_write> vsc_stats:array<atomic<u32>,8>;
@group(0) @binding(48) var<storage,read_write> vsc_dispatch:array<u32,8>;
// The insert passes read the frame's finished counts (the probe's and walks' vsm_output).
@group(0) @binding(52) var vsc_counts:texture_2d<u32>;
// Two claim words per table slot (vsc_claim): the highest priority, and the complement of the
// lowest. Zero between frames: each slot's winner clears them.
@group(0) @binding(53) var<storage,read_write> vsc_claims:array<atomic<u32>>;
const VSC_SHOWN:u32=1u<<18u;
// The consensus neighbourhood's radius in pixels (1: 3x3).
override VSC_CONSENSUS:u32=1u;
// Cell edge in receiver pixels (a power of two keeps cell and map levels in step).
override VSC_CELL:f32=1.0;
override VSC_STATS:bool=false;
// Frames a turning sun may age a cell (N8e's rolling refresh bound).
const VSC_LAG:u32=16u;
const VSC_SLOTS:u32=4u;

struct VscCell {level:i32,coords:vec3<i32>,size:f32};
struct VscKey {bucket:u32,fingerprint:u32,check:u32,phase:u32};

// The cell of a receiver point at its footprint.
fn vsc_cell(world:vec3<f32>)->VscCell {
    let footprint=length(world-vsm.eye.xyz)*vsm.eye.w;
    let base=vsm.params.y;
    let level=i32(floor(log2(max(footprint*VSC_CELL,1e-12)/base)));
    let size=base*exp2(f32(level));
    return VscCell(level,vec3<i32>(floor(world/size)),size);
}
fn vsc_centre(c:VscCell)->vec3<f32> {return (vec3<f32>(c.coords)+vec3<f32>(0.5))*c.size;}
// Cells are certified by their pixels' neighbourhoods across triangles, so they need not belong
// to one: at the quarry's LOD triangles of about a pixel, a sub-pixel camera step moves most
// pixels onto another triangle (N8n, measured: 59% shown while moving with the triangle in the
// key, 86% without).
fn vsc_chain(seed:u32,slot:u32,side:u32,level:i32,coords:vec3<i32>)->u32 {
    var h=vsm_hash(seed^slot);
    h=vsm_hash(h^(u32(level+128)|(side<<8u)));
    h=vsm_hash(h^u32(coords.x));
    h=vsm_hash(h^u32(coords.y));
    return vsm_hash(h^u32(coords.z));
}
// The bucket hashes the cell's 4x4x4-cell world block and places the block's 64 cells in 64
// consecutive buckets (2 KiB), so a tile's probes share cache lines; an independent chain over
// the cell is its fingerprint (52 bits name a cell). Refresh phases are per world block of
// 2^VSC_PHASE_BLOCK cells a side, so a turning sun's refreshes stay coherent on screen.
override VSC_PHASE_BLOCK:u32=2u;
fn vsc_key(slot:u32,side:u32,c:VscCell)->VscKey {
    let block=c.coords>>vec3<u32>(2u);
    let local=vec3<u32>(c.coords&vec3<i32>(3));
    let h=vsc_chain(0x1234567u,slot,side,c.level,block);
    let f=vsc_chain(0x7654321u,slot,side,c.level,c.coords);
    let bucket=((h<<6u)|local.x|(local.y<<2u)|(local.z<<4u))&vsc.buckets;
    var phase=h>>24u;
    if VSC_PHASE_BLOCK!=2u {phase=vsc_chain(0x2468aceu,slot,side,c.level,c.coords>>vec3<u32>(VSC_PHASE_BLOCK));}
    return VscKey(bucket,max(f,1u),h>>28u,phase&(VSC_LAG-1u));
}
// The opaque receiver's viewed side of its plane (the same for every point of the plane).
fn vsc_side(world:vec3<f32>,geometric:vec3<f32>,thin:bool)->u32 {
    return select(u32(dot(geometric,lighting.eye-world)>=0.0),2u,thin);
}
// The radius around a cell's centre that its value depends on: its cube, and for a shown cell
// the neighbourhood that certified it (margin exponent m: c.size * 2^m covers every neighbour).
fn vsc_extent(c:VscCell,value:u32)->f32 {
    let cube=0.8660254*c.size;
    if (value&VSC_SHOWN)==0u {return cube;}
    return max(cube,c.size*exp2(f32((value>>1u)&7u)));
}

// N8f's reach test (vsm_reached) for every point within `extent` of `centre` at once: their
// march neighbourhoods at every level from the cell's recorded absolute start level against the
// moved casters' light-space bounds. Each point's radius at a level is at most the radius at the
// region's lowest height, and its march stops no later; so the test is conservative for the walk
// of every receiver the cell's value or verdict came from, and depends on the cell alone. True
// when a moved caster may change the cell.
fn vsc_reached(centre:vec3<f32>,extent:f32,recorded:u32)->bool {
    if vsm.counts.w==0u {return false;}
    let finest=u32(i32(vsm.levels[0].info.y)&15);
    let start=(recorded+16u-finest)&15u;
    if start>=vsm.counts.x {return true;}
    let c=vsm_light(centre);
    let low=c.z-extent;
    for(var l=start;l<vsm.counts.x;l+=1u) {
        let r=vec2<f32>(vsm_radius(l,low)+extent);
        for(var i=0u;i<vsm.counts.w;i+=1u) {
            let m=vsm.rects[i];
            if all(c.xy+r>=m.xy) && all(c.xy-r<=m.zw) {return true;}
        }
        if vsm_cap(l)>=vsm.depth.w-low {break;}
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
fn vsc_shadowed(record:vec4<u32>,world:vec4<f32>,normal:vec4<f32>)->bool {
    let light=lights[0];
    if record.x==0u || normal.w==0.0 || light.kind.z==0u {return false;}
    let depth=dot(shadow_views[light.kind.y].camera_depth,vec4<f32>(world.xyz,1.0));
    for(var i=0u;i<light.kind.z;i+=1u) {
        let coverage=shadow_views[light.kind.y+i].coverage_bias;
        if depth>=coverage.x && depth<=coverage.y {return true;}
    }
    return false;
}

// vsc_find's results that are not a shown count: absent; present but due (stale for the sun or
// the casters) with its last work; current but not shown, with its last work. Work is the low
// three bits.
const VSC_MISS:u32=0xffffffffu;
const VSC_STALE:u32=0xfffffff0u;
const VSC_UNSHOWN:u32=0xffffffe0u;
fn vsc_work(e:vec2<u32>)->u32 {return (e.y>>19u)&7u;}
// A slot's value is current: computed at this sun epoch and caster epoch.
fn vsc_current(e:vec2<u32>)->bool {
    return e.x!=0u && ((e.y>>12u)&63u)==(vsc.casters&63u) && (e.y>>22u)==(vsc.sun&1023u);
}
// A cell's cached visible count, VSC_UNSHOWN | its work, VSC_STALE | its work or VSC_MISS. On a
// caster-moving frame a kept hit is marked current (its caster epoch rewritten). Where the moved
// casters reach a shown cell's certifying neighbourhood but not its cube, its count still holds
// and it is kept unshown, so its pixels walk once the camera moves. Every pixel of the cell
// decides alike (vsc_reached), so the rewrite is the same whichever writes it.
fn vsc_find(c:VscCell,slot:u32,side:u32)->u32 {
    let key=vsc_key(slot,side,c);
    let base=key.bucket*VSC_SLOTS;
    let moved=(vsc.flags&1u)!=0u;
    for(var s=0u;s<VSC_SLOTS;s+=1u) {
        let e=vsc_table[base+s];
        if e.x!=key.fingerprint || ((e.y>>8u)&15u)!=key.check {continue;}
        let lag=((vsc.sun&1023u)-(e.y>>22u))&1023u;
        if lag!=0u && (lag>=VSC_LAG || ((vsc.frame+key.phase)&(VSC_LAG-1u))==0u) {
            if VSC_STATS {atomicAdd(&vsc_stats[3],1u);}
            return VSC_STALE|vsc_work(e);
        }
        let epoch=(e.y>>12u)&63u;
        let current=vsc.casters&63u;
        var value=e.y;
        // One caster-moving frame behind: still valid where the moved casters cannot reach.
        if epoch!=current {
            if !moved || epoch!=((current+63u)&63u) {return VSC_STALE|vsc_work(e);}
            let centre=vsc_centre(c);
            let recorded=(value>>4u)&15u;
            if vsc_reached(centre,0.8660254*c.size,recorded) {
                if VSC_STATS {atomicAdd(&vsc_stats[2],1u);}
                return VSC_STALE|vsc_work(e);
            }
            value=(value&~(63u<<12u))|(current<<12u);
            if (value&VSC_SHOWN)!=0u && vsc_reached(centre,vsc_extent(c,value),recorded) {value&=~(VSC_SHOWN|15u);}
            vsc_table[base+s]=vec2<u32>(e.x,value);
        }
        if (value&VSC_SHOWN)==0u {
            if VSC_STATS {atomicAdd(&vsc_stats[4],1u);}
            return VSC_UNSHOWN|((value>>19u)&7u);
        }
        return select(0u,vsm_total(),(value&1u)!=0u);
    }
    return VSC_MISS;
}

var<workgroup> vsc_wg_misses:atomic<u32>;
var<workgroup> vsc_wg_base:u32;
var<workgroup> vsc_wg_buckets:array<atomic<u32>,8>;
// The walk order (N8m's heaviest first, applied to the queue): the probe queues each walking
// pixel with its cell's last work estimate as a bucket, and a queue of at least VSC_SORT_MIN
// entries (more than a wave of resident lanes) is copied after itself in bucket order, heaviest
// first, so the slowest walks start first. VSC_SORT buckets: 2, whether the estimate reaches
// VSC_HEAVY; 8, its bit length; 1, none. Pixels with no estimate (absent cells) take
// VSC_UNKNOWN. Measured (docs/non-rt-n8o.md): 2 buckets beat 8 on camera and caster frames, and
// a turning sun's queue, about one wave, walks fastest in the probe's order (its spatial
// coherence): ordered, it took 0.66-0.79 ms against 0.57-0.60.
override VSC_SORT:u32=2u;
override VSC_SORT_MIN:u32=32768u;
override VSC_HEAVY:u32=5u;
override VSC_UNKNOWN:u32=3u;
fn vsc_bucket(visible:u32)->u32 {
    let work=select(visible&7u,VSC_UNKNOWN,visible==VSC_MISS);
    if VSC_SORT>=8u {return work;}
    if VSC_SORT==2u {return u32(work>=VSC_HEAVY);}
    return 0u;
}
// The queue's tail (32 words before the buffer's end): per bucket its count (probe), its first
// entry in the sorted copy (vsc_args) and its fill cursor (vsc_sort); the last word is the walk's
// chunk cursor. Entries are two words each, after the 4-word header.
fn vsc_tail()->u32 {return arrayLength(&vsm_scratch.words)-32u;}
fn vsc_capacity()->u32 {return (arrayLength(&vsm_scratch.words)-36u)/2u;}
var<workgroup> vsc_wg_hits:atomic<u32>;
var<workgroup> vsc_wg_probes:atomic<u32>;
// The probe's workgroup in pixels (multiples of the 8x4 trace tile); the encoder reads them from
// the built constants. Measured: one tile a workgroup spent 18% of the probe on its coverage load.
override VSC_PROBE_X:u32=16u;
override VSC_PROBE_Y:u32=8u;
// Every covered pixel of the frame, VSC_PROBE_X x VSC_PROBE_Y a workgroup.
@compute @workgroup_size(VSC_PROBE_X,VSC_PROBE_Y) fn vsc_probe(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    if lane==0u {atomicStore(&vsc_wg_misses,0u);atomicStore(&vsc_wg_hits,0u);atomicStore(&vsc_wg_probes,0u);}
    if lane<8u {atomicStore(&vsc_wg_buckets[lane],0u);}
    workgroupBarrier();
    let size=textureDimensions(vsm_world);
    let id=gid.xy;
    let inside=all(id<size) && textureLoad(vsm_tiles,id/VSM_TRACE_TILE,0).x!=0u;
    var miss=false;
    var bucket=0u;
    if inside {
        let p=vec2<i32>(id);
        let record=textureLoad(vsm_hit,p,0);
        let world=textureLoad(vsm_world,p,0);
        let normal=textureLoad(vsm_normal,p,0);
        if !vsc_shadowed(record,world,normal) {
            textureStore(vsm_output,p,vec4<u32>(256u));
        } else {
            let side=vsc_side(world.xyz,normal.xyz,normal.w<0.0);
            let visible=vsc_find(vsc_cell(world.xyz),record.x-1u,side);
            if VSC_STATS {
                atomicAdd(&vsc_wg_probes,1u);
                if visible<VSC_UNSHOWN {atomicAdd(&vsc_wg_hits,1u);}
            }
            // A still frame keeps a walking pixel's last count (its cell is current: neither
            // the sun since its refresh nor anything else changed it), unless the pixel lies in
            // a moved caster's old or new screen bounds, where its receiver may have changed
            // (N8f's screen rectangles; measured without them: 533-778 pixels differed from a
            // fresh lookup after 48 caster-moving frames).
            let unshown=(visible&0xfffffff0u)==VSC_UNSHOWN;
            let still=(vsc.flags&2u)!=0u && unshown && !vsc_in_moved(id);
            if visible>=VSC_UNSHOWN {miss=!still;} else {vsm_visible(id,visible);}
            bucket=vsc_bucket(visible);
        }
    }
    var at=0u;
    if miss {
        at=atomicAdd(&vsc_wg_misses,1u);
        atomicAdd(&vsc_wg_buckets[bucket],1u);
    }
    workgroupBarrier();
    if lane==0u {
        let n=atomicLoad(&vsc_wg_misses);
        if n!=0u {vsc_wg_base=atomicAdd(&vsm_scratch.queue,n);}
        if VSC_STATS {atomicAdd(&vsc_stats[0],atomicLoad(&vsc_wg_probes));atomicAdd(&vsc_stats[1],atomicLoad(&vsc_wg_hits));}
    }
    if lane<8u {
        let n=atomicLoad(&vsc_wg_buckets[lane]);
        if n!=0u {atomicAdd(&vsm_scratch.words[vsc_tail()+lane],n);}
    }
    workgroupBarrier();
    if miss {
        // Word 2k+1 holds the bucket until the walk writes its result (bit 31).
        let k=vsc_wg_base+at;
        atomicStore(&vsm_scratch.words[2u*k],id.x|(id.y<<16u));
        atomicStore(&vsm_scratch.words[2u*k+1u],bucket);
    }
}

// The queued walk's lanes per miss (as vmap_queue_args: a small queue spreads each miss over up to
// VSC_SPREAD lanes, up to VSC_LANES lanes in all), the sorted copy's bucket offsets (scratch pad0:
// the copy's first entry, n; 0 when unsorted), then the dispatches of the walk and of the sort
// and insert passes. N8m's schedule: VSC_PERSIST workgroups (0: one per 32 lanes) take 32-lane
// chunks from a cursor, the queue buffer's last word, zeroed with the tail by the encoder.
override VSC_SPREAD:u32=8u;
override VSC_LANES:u32=8192u;
override VSC_PERSIST:u32=4096u;
// P-NSQ3: on a frame of several direction batches (P-NSQ's budget), each queued pixel takes at
// least this many lanes (a power of two, at most 32), and its lanes split its batches
// (vsc_walk). One lane walking all 32 batches of a cut held its warp until its pixel's last
// batch; a pixel's batches cost about alike, so split lanes finish together.
override VSC_BATCH_LANES:u32=8u;
// The queued walk's lanes per pixel: vsc_args' spread, and with persistent workgroups on a
// several-batch frame at least VSC_BATCH_LANES, rounded down to a power of two so that a pixel's
// lanes share a 32-lane chunk. (vsc_args' dispatch may then have fewer workgroups than lanes;
// persistent workgroups take chunks until vsc_missed's own lane count is covered.)
fn vsc_spread()->u32 {
    let spread=max(vsm_scratch.spread,1u);
    if VSC_PERSIST==0u || vsm.nsq.x<=1u {return spread;}
    return max(1u<<(31u-countLeadingZeros(spread)),min(vsm.nsq.x,VSC_BATCH_LANES));
}
// A pixel's batches split over this many of its lanes.
fn vsc_split()->u32 {return select(min(vsc_spread(),vsm.nsq.x),1u,VSC_PERSIST==0u);}
fn vsc_cursor()->u32 {return arrayLength(&vsm_scratch.words)-1u;}
@compute @workgroup_size(1) fn vsc_args() {
    let n=atomicLoad(&vsm_scratch.queue);
    var first=0u;
    if VSC_SORT>1u && n>=VSC_SORT_MIN && 2u*n<=vsc_capacity() {
        first=n;
        var at=0u;
        for(var b=8u;b>0u;b-=1u) {
            atomicStore(&vsm_scratch.words[vsc_tail()+8u+b-1u],at);
            at+=atomicLoad(&vsm_scratch.words[vsc_tail()+b-1u]);
        }
    }
    vsm_scratch.pad0=first;
    let spread=clamp(VSC_LANES/max(n,1u),1u,max(VSC_SPREAD,1u));
    vsm_scratch.spread=spread;
    let groups=(n*spread+31u)/32u;
    vsc_dispatch[0]=select(groups,min(groups,VSC_PERSIST),VSC_PERSIST!=0u);vsc_dispatch[1]=1u;vsc_dispatch[2]=1u;
    vsc_dispatch[4]=(n+63u)/64u;vsc_dispatch[5]=1u;vsc_dispatch[6]=1u;
}

var<workgroup> vsc_wg_sort:array<atomic<u32>,8>;
var<workgroup> vsc_wg_sort_base:array<u32,8>;
// Copies queue entry i to its bucket's place in the sorted copy (pad0 != 0). Each workgroup takes
// its places per bucket at once (same-address atomics serialize), so a bucket keeps the probe's
// order of workgroups.
@compute @workgroup_size(64) fn vsc_sort(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    if li<8u {atomicStore(&vsc_wg_sort[li],0u);}
    workgroupBarrier();
    let first=vsm_scratch.pad0;
    let inside=first!=0u && gid.x<atomicLoad(&vsm_scratch.queue);
    var b=0u;
    var at=0u;
    if inside {
        b=atomicLoad(&vsm_scratch.words[2u*gid.x+1u])&7u;
        at=atomicAdd(&vsc_wg_sort[b],1u);
    }
    workgroupBarrier();
    if li<8u {
        let n=atomicLoad(&vsc_wg_sort[li]);
        if n!=0u {vsc_wg_sort_base[li]=atomicAdd(&vsm_scratch.words[vsc_tail()+16u+li],n);}
    }
    workgroupBarrier();
    if inside {
        let k=first+atomicLoad(&vsm_scratch.words[vsc_tail()+8u+b])+vsc_wg_sort_base[b]+at;
        atomicStore(&vsm_scratch.words[2u*k],atomicLoad(&vsm_scratch.words[2u*gid.x]));
        atomicStore(&vsm_scratch.words[2u*k+1u],0u);
    }
}
// The pixel this lane finishes after its chunk's barrier (vsc_finish): its queue entry (VSM_NONE:
// none), point and march level.
var<private> vsc_entry:u32;
var<private> vsc_pixel:vec2<u32>;
var<private> vsc_level:u32;
// The exact walk for queue lane g (each queued pixel on `spread` lanes), with Exact's setup
// (vsm_prepare: the pixel's receiver point, directions and per-pixel jitter). The pixel's first
// vsc_split() lanes walk every vsc_split()-th batch from their own and leave their visible count
// and work in their slots of vsm_wg_dirs (directions no longer needed), as exact floats: counts
// are at most 256, and work is clamped where only its bit length up to 15 is read.
fn vsc_walk(g:u32) {
    vsc_entry=VSM_NONE;
    vsm_wg_dirs[0][vsm_lane]=0.0;vsm_wg_dirs[1][vsm_lane]=0.0;
    let spread=vsc_spread();
    let split=vsc_split();
    let lane=g%spread;
    if lane>=split {return;}
    let slot_k=g/spread;
    if slot_k>=atomicLoad(&vsm_scratch.queue) {return;}
    let k=vsm_scratch.pad0+slot_k;
    let word=atomicLoad(&vsm_scratch.words[2u*k]);
    let id=vec2<u32>(word&0xffffu,word>>16u);
    let px=vsm_prepare(id);
    if !px.valid {
        if lane==0u {textureStore(vsm_output,vec2<i32>(id),vec4<u32>(256u));}
        return;
    }
    let visible=vsm_walk_batches(px,lane,split);
    vsm_wg_dirs[0][vsm_lane]=f32(visible);
    vsm_wg_dirs[1][vsm_lane]=f32(min(vsm_batch_work,1u<<20u));
    if lane==0u {vsc_entry=k;vsc_pixel=id;vsc_level=px.level;}
}
// After the chunk's barrier: a pixel's first lane sums its lanes' counts and work and writes the
// count, and word 2k+1 gets bit 31, the verdict's code, the start level << 4 and the work's bit
// length << 8 for vsc_claim.
fn vsc_finish() {
    let k=vsc_entry;
    if k==VSM_NONE {return;}
    var visible=0u;
    var total=0u;
    for(var i=0u;i<vsc_split();i+=1u) {
        visible+=u32(vsm_wg_dirs[0][vsm_lane+i]);
        total+=u32(vsm_wg_dirs[1][vsm_lane+i]);
    }
    vsm_visible(vsc_pixel,visible);
    // The verdict's visible code (0..n8e.x): over several batches (P-NSQ), 0 or n8e.x when
    // solid and 1 when partial (only solidity and the solid count are read).
    var code=visible;
    if vsm_batches()>1u {code=select(select(1u,vsm.n8e.x,visible==vsm_total()),0u,visible==0u);}
    let start=u32((i32(vsm.levels[0].info.y)+i32(min(vsc_level,15u)))&15);
    let work=min(15u,32u-countLeadingZeros(total));
    atomicStore(&vsm_scratch.words[2u*k+1u],0x80000000u|code|(start<<4u)|(work<<8u));
}
var<workgroup> vsc_wg_chunk:u32;
// The queued pixels, 32 lanes a workgroup: one chunk each (VSC_PERSIST 0), or chunks from the
// cursor until the queue's lanes are taken. A chunk is claimed only after the last one is done,
// whose barrier also keeps the next chunk's directions from overwriting the slots vsc_finish read.
@compute @workgroup_size(32) fn vsc_missed(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    vsm_lane=li;
    if VSC_PERSIST==0u {
        vsc_walk(gid.x);
        workgroupBarrier();
        vsc_finish();
        return;
    }
    let lanes=atomicLoad(&vsm_scratch.queue)*vsc_spread();
    loop {
        if li==0u {vsc_wg_chunk=atomicAdd(&vsm_scratch.words[vsc_cursor()],1u);}
        let t=workgroupUniformLoad(&vsc_wg_chunk);
        if t*32u>=lanes {break;}
        vsc_walk(t*32u+li);
        workgroupBarrier();
        vsc_finish();
    }
}

// ---- Inserting the frame's walks (N8o): claim, then resolve ----
// A queued result's pixel, cell and key, recomputed alike by both passes.
struct VscResult {valid:bool,p:vec2<i32>,world:vec3<f32>,slot:u32,side:u32,cell:VscCell,key:VscKey};
fn vsc_result(entry:u32)->VscResult {
    var out:VscResult;
    let word=atomicLoad(&vsm_scratch.words[2u*entry]);
    out.p=vec2<i32>(vec2<u32>(word&0xffffu,word>>16u));
    let record=textureLoad(vsm_hit,out.p,0);
    let world=textureLoad(vsm_world,out.p,0);
    let normal=textureLoad(vsm_normal,out.p,0);
    out.valid=vsc_shadowed(record,world,normal);
    out.world=world.xyz;
    out.slot=record.x-1u;
    out.side=vsc_side(world.xyz,normal.xyz,normal.w<0.0);
    out.cell=vsc_cell(world.xyz);
    out.key=vsc_key(out.slot,out.side,out.cell);
    return out;
}
// A claim's priority: direct (the pixel's own cell) above a fill (VSC_SPLAT), then the verdict
// grade (2 fully lit, 1 fully blocked, both certified by the pixel's neighbourhood within margin;
// 0 neither), the margin exponent, then the pixel (unique).
fn vsc_priority(direct:bool,grade:u32,margin:u32,p:vec2<i32>)->u32 {
    let size=textureDimensions(vsm_world);
    return (u32(direct)<<31u)|(grade<<29u)|(margin<<26u)|(u32(p.x)+u32(p.y)*size.x+1u);
}
fn vsc_grade(priority:u32)->u32 {return (priority>>29u)&3u;}
// The smallest margin exponent m with size * 2^m >= far, or 8 when none up to 7 is.
fn vsc_margin(size:f32,far:f32)->u32 {
    var m=0u;
    while m<8u && size*exp2(f32(m))<far {m+=1u;}
    return m;
}
// The verdict of pixel p's walk with `visible` of k directions open: grade 2 or 1 when the walk
// is solid and every receiver within VSC_CONSENSUS pixels finished this frame with its count,
// and the farthest of them, or of the cubes their counts came from, is within c.size * 2^margin
// (margin at most 7) of the cell's centre; else grade 0. z: that farthest distance in quarters of
// the cell's edge, rounded up, or 63 when it exceeds 15.75 (the fills' margins start from it,
// alike in both passes; 63 makes none).
fn vsc_verdict(p:vec2<i32>,c:VscCell,visible:u32)->vec3<u32> {
    let k=vsm.n8e.x;
    if visible!=k && visible!=0u {return vec3<u32>(0u);}
    let count=(256u*visible+k/2u)/k;
    let size=vec2<i32>(textureDimensions(vsc_counts));
    let centre=vsc_centre(c);
    let r=i32(VSC_CONSENSUS);
    var far=0.0;
    for(var y=-r;y<=r;y+=1) {
        for(var x=-r;x<=r;x+=1) {
            let q=p+vec2<i32>(x,y);
            // Uncovered tiles hold no hit records or counts of this frame (vsm_covered).
            if any(q<vec2<i32>(0)) || any(q>=size) || !vsm_covered(vec2<u32>(q)) {continue;}
            if textureLoad(vsc_counts,q,0).x!=count {return vec3<u32>(0u);}
            // The neighbour's count may be its own cell's, computed anywhere in that cube, whose
            // edge is at most VSC_CELL footprints (vsc_cell).
            let wq=textureLoad(vsm_world,q,0).xyz;
            far=max(far,length(wq-centre)+1.7320508*VSC_CELL*length(wq-vsm.eye.xyz)*vsm.eye.w);
        }
    }
    let margin=vsc_margin(c.size,far);
    if margin==8u {
        if VSC_STATS {atomicAdd(&vsc_stats[5],1u);}
        return vec3<u32>(0u);
    }
    return vec3<u32>(select(1u,2u,visible==k),margin,u32(min(ceil(far/c.size*4.0),63.0)));
}
// The slot a result writes, from the table as this frame's probe left it (vsc_resolve writes
// it only after every claim): its cell's own, else the first slot that is empty or not current
// from one the fingerprint picks, else (unless `fill`) that one. A fill never replaces a current
// value: VSC_SLOTS * buckets when it would. y: the slot holds the cell current and unshown.
fn vsc_target(key:VscKey,fill:bool)->vec2<u32> {
    let base=key.bucket*VSC_SLOTS;
    let pick=(key.fingerprint>>8u)&(VSC_SLOTS-1u);
    let none=VSC_SLOTS*(vsc.buckets+1u);
    var free=VSC_SLOTS;
    for(var s=0u;s<VSC_SLOTS;s+=1u) {
        let e=vsc_table[base+s];
        if e.x==key.fingerprint && ((e.y>>8u)&15u)==key.check {
            let current=vsc_current(e);
            return vec2<u32>(select(base+s,none,fill && current),u32(current && (e.y&VSC_SHOWN)==0u));
        }
        let t=(pick+s)&(VSC_SLOTS-1u);
        if free==VSC_SLOTS && !vsc_current(vsc_table[base+t]) {free=t;}
    }
    if free==VSC_SLOTS {return vec2<u32>(select(base+pick,none,fill),0u);}
    return vec2<u32>(base+free,0u);
}
// A certified result also fills the cells of the midpoints to its pixel's 3x3 neighbours on the
// same placement and side: they lie inside the neighbourhood that agreed, where the next frames'
// receiver points land (without them, measured: 81% of receivers shown while the camera moves,
// against 86%). A fill claims below any pixel's own result, only where no current value is, and
// its margin covers the source's neighbourhood from the fill's centre. Fills are made only on
// frames that move the camera: until it moves, no receiver point lands in them.
override VSC_SPLAT:bool=true;
struct VscFill {valid:bool,key:VscKey,margin:u32};
// Neighbour n's fill (n of the 3x3, 4 the pixel itself), from its world point.
fn vsc_fill_at(x:VscResult,wq:vec3<f32>,quarters:u32)->VscFill {
    var out:VscFill;
    let c=vsc_cell((x.world+wq)*0.5);
    out.key=vsc_key(x.slot,x.side,c);
    if out.key.bucket==x.key.bucket && out.key.fingerprint==x.key.fingerprint {return out;}
    let far=f32(quarters)*x.cell.size/4.0+length(vsc_centre(c)-vsc_centre(x.cell));
    out.margin=vsc_margin(c.size,far);
    out.valid=out.margin<8u;
    return out;
}
fn vsc_neighbour(x:VscResult,n:u32)->vec2<i32> {return x.p+vec2<i32>(i32(n%3u)-1,i32(n/3u)-1);}
// The fill mask's bit for neighbour n (the 3x3 without its centre).
fn vsc_fill_bit(n:u32)->u32 {return 1u<<(n-u32(n>4u));}
// Claims each queued result's slot (unless the slot already holds its cell current and unshown:
// its verdict cannot change), and its fills'. Word 2k+1 becomes the result with its verdict:
// bit 31, the visible count, start level << 4, work << 8 (3 bits), bit 11 when the slot was kept,
// grade << 12, margin << 14, the farthest neighbour in quarters << 17 and the claimed fills' mask
// << 23 (vsc_fill_bit).
@compute @workgroup_size(64) fn vsc_claim(@builtin(global_invocation_id) gid:vec3<u32>) {
    if gid.x>=atomicLoad(&vsm_scratch.queue) {return;}
    let k=vsm_scratch.pad0+gid.x;
    let result=atomicLoad(&vsm_scratch.words[2u*k+1u]);
    if (result&0x80000000u)==0u {return;}
    let x=vsc_result(k);
    if !x.valid {return;}
    let verdict=vsc_verdict(x.p,x.cell,result&15u);
    let aim=vsc_target(x.key,false);
    if aim.y==0u {
        let priority=vsc_priority(true,verdict.x,verdict.y,x.p);
        atomicMax(&vsc_claims[2u*aim.x],priority);
        atomicMax(&vsc_claims[2u*aim.x+1u],~priority);
    }
    var mask=0u;
    if VSC_SPLAT && verdict.x!=0u && verdict.z<63u && (vsc.flags&2u)==0u {
        let size=vec2<i32>(textureDimensions(vsm_world));
        for(var n=0u;n<9u;n+=1u) {
            let q=vsc_neighbour(x,n);
            if n==4u || any(q<vec2<i32>(0)) || any(q>=size) || !vsm_covered(vec2<u32>(q)) {continue;}
            let wq=textureLoad(vsm_world,q,0);
            let nq=textureLoad(vsm_normal,q,0);
            if textureLoad(vsm_hit,q,0).x!=x.slot+1u || vsc_side(wq.xyz,nq.xyz,nq.w<0.0)!=x.side {continue;}
            let f=vsc_fill_at(x,wq.xyz,verdict.z);
            if !f.valid {continue;}
            let fill=vsc_target(f.key,true).x;
            if fill==VSC_SLOTS*(vsc.buckets+1u) {continue;}
            if VSC_STATS {atomicAdd(&vsc_stats[6],1u);}
            let p=vsc_priority(false,verdict.x,f.margin,x.p);
            atomicMax(&vsc_claims[2u*fill],p);
            atomicMax(&vsc_claims[2u*fill+1u],~p);
            mask|=vsc_fill_bit(n);
        }
    }
    let work=min((result>>8u)&15u,7u)<<8u;
    atomicStore(&vsm_scratch.words[2u*k+1u],(result&0x800000ffu)|work|(aim.y<<11u)|(verdict.x<<12u)|(verdict.y<<14u)|(verdict.z<<17u)|(mask<<23u));
}
// The slot of `key`'s bucket whose highest claim is `priority`, after clearing its claims, and
// the grade of its lowest claim; VSC_SLOTS when this claim did not win.
fn vsc_won(key:VscKey,priority:u32)->vec2<u32> {
    let base=key.bucket*VSC_SLOTS;
    var slot=VSC_SLOTS;
    for(var s=0u;s<VSC_SLOTS;s+=1u) {
        if atomicLoad(&vsc_claims[2u*(base+s)])==priority {slot=base+s;}
    }
    if slot==VSC_SLOTS {return vec2<u32>(VSC_SLOTS,0u);}
    let lowest=~atomicLoad(&vsc_claims[2u*slot+1u]);
    atomicStore(&vsc_claims[2u*slot],0u);
    atomicStore(&vsc_claims[2u*slot+1u],0u);
    return vec2<u32>(slot,vsc_grade(lowest));
}
// The highest claim on each slot writes it. The cell is shown when every claim on the slot had
// its grade (all its pixels walked to the same solid count, each certified by its
// neighbourhood, and every fill agreed), unless the slot already holds it current and not
// shown: a cell once seen beside a differing count stays unshown until it is due again. The
// winner clears the claims.
@compute @workgroup_size(64) fn vsc_resolve(@builtin(global_invocation_id) gid:vec3<u32>) {
    if gid.x>=atomicLoad(&vsm_scratch.queue) {return;}
    let k=vsm_scratch.pad0+gid.x;
    let result=atomicLoad(&vsm_scratch.words[2u*k+1u]);
    let kept=(result&0x800u)!=0u;
    let mask=(result>>23u)&0xffu;
    if (result&0x80000000u)==0u || (kept && mask==0u) {return;}
    let x=vsc_result(k);
    if !x.valid {return;}
    let grade=(result>>12u)&3u;
    let margin=(result>>14u)&7u;
    let epochs=((vsc.casters&63u)<<12u)|(((result>>8u)&7u)<<19u)|((vsc.sun&1023u)<<22u)|(result&0xf0u);
    if !kept {
        let won=vsc_won(x.key,vsc_priority(true,grade,margin,x.p));
        if won.x!=VSC_SLOTS {
            let shown=grade!=0u && won.y==grade;
            let field=select(0u,u32(grade==2u)|(margin<<1u),shown);
            vsc_table[won.x]=vec2<u32>(x.key.fingerprint,field|(x.key.check<<8u)|epochs|select(0u,VSC_SHOWN,shown));
        }
    }
    for(var n=0u;n<9u;n+=1u) {
        if n==4u || (mask&vsc_fill_bit(n))==0u {continue;}
        let f=vsc_fill_at(x,textureLoad(vsm_world,vsc_neighbour(x,n),0).xyz,(result>>17u)&63u);
        let fill=vsc_won(f.key,vsc_priority(false,grade,f.margin,x.p));
        if fill.x==VSC_SLOTS {continue;}
        // Fills that disagree leave the cell unshown. A fill's work estimate is its source's.
        let shown=fill.y==grade;
        let field=select(0u,u32(grade==2u)|(f.margin<<1u)|VSC_SHOWN,shown);
        vsc_table[fill.x]=vec2<u32>(f.key.fingerprint,field|(f.key.check<<8u)|epochs);
    }
}
