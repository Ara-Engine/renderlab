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
// (vsm_scratch, the hybrid's layout: count, spread, then two words per pixel; walks known to be
// heavy from the top end); vsc_args sizes the next three dispatches; vsc_missed runs the exact
// walk for the queue, heaviest first; vsc_claim and vsc_resolve store the results.
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
//     cell's cube and, for a shown cell, to the neighbourhood that certified it (its margin),
//     clears it. The walk at any point of that region, and so the cell's count and its verdict,
//     cannot have changed (N8o). A cell nobody looked at on a caster-moving frame becomes stale.
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
//     exponent (bits 1-3, vsc_margin); an unshown cell's last walk work, as its bit length (bits
//     0-2, at most 7; N8m's measure, the walk order's estimate);
//   bits 4-7: the march's absolute start level & 15;
//   bits 8-11: a check nibble of the bucket hash;
//   bits 12-17: the caster epoch & 63;
//   bit 18: shown;
//   bits 19-31: the sun epoch & 8191.
@group(0) @binding(50) var<storage,read_write> vsc_table:array<vec2<u32>>;
// VSC_STATS screens: probed receivers, hits, cells the caster test retired, refreshes due to the
// sun, cells found but not shown.
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
// the cell is its fingerprint (52 bits name a cell). Refresh phases are per block too, so a
// turning sun's refreshes stay coherent.
fn vsc_key(slot:u32,side:u32,c:VscCell)->VscKey {
    let block=c.coords>>vec3<u32>(2u);
    let local=vec3<u32>(c.coords&vec3<i32>(3));
    let h=vsc_chain(0x1234567u,slot,side,c.level,block);
    let f=vsc_chain(0x7654321u,slot,side,c.level,c.coords);
    let bucket=((h<<6u)|local.x|(local.y<<2u)|(local.z<<4u))&vsc.buckets;
    return VscKey(bucket,max(f,1u),h>>28u,(h>>24u)&(VSC_LAG-1u));
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
// three bits (0 for a shown cell).
const VSC_MISS:u32=0xffffffffu;
const VSC_STALE:u32=0xfffffff0u;
const VSC_UNSHOWN:u32=0xffffffe0u;
fn vsc_work(e:vec2<u32>)->u32 {return select(e.y&7u,0u,(e.y&VSC_SHOWN)!=0u);}
// A slot's value is current: computed at this sun epoch and caster epoch.
fn vsc_current(e:vec2<u32>)->bool {
    return e.x!=0u && ((e.y>>12u)&63u)==(vsc.casters&63u) && (e.y>>19u)==(vsc.sun&8191u);
}
// A cell's cached visible count, VSC_UNSHOWN | its work, VSC_STALE | its work or VSC_MISS. On a
// caster-moving frame a kept hit is marked current (its caster epoch rewritten): every pixel of
// the cell decides alike (vsc_reached), so the rewrite is the same whichever writes it.
fn vsc_find(c:VscCell,slot:u32,side:u32)->u32 {
    let key=vsc_key(slot,side,c);
    let base=key.bucket*VSC_SLOTS;
    let moved=(vsc.flags&1u)!=0u;
    for(var s=0u;s<VSC_SLOTS;s+=1u) {
        let e=vsc_table[base+s];
        if e.x!=key.fingerprint || ((e.y>>8u)&15u)!=key.check {continue;}
        let lag=((vsc.sun&8191u)-(e.y>>19u))&8191u;
        if lag!=0u && (lag>=VSC_LAG || ((vsc.frame+key.phase)&(VSC_LAG-1u))==0u) {
            if VSC_STATS {atomicAdd(&vsc_stats[3],1u);}
            return VSC_STALE|vsc_work(e);
        }
        let epoch=(e.y>>12u)&63u;
        let current=vsc.casters&63u;
        var keep=epoch==current;
        // One caster-moving frame behind: still valid where the moved casters cannot reach.
        if !keep && moved && epoch==((current+63u)&63u) {
            if vsc_reached(vsc_centre(c),vsc_extent(c,e.y),(e.y>>4u)&15u) {
                if VSC_STATS {atomicAdd(&vsc_stats[2],1u);}
            } else {
                vsc_table[base+s]=vec2<u32>(e.x,(e.y&~(63u<<12u))|(current<<12u));
                keep=true;
            }
        }
        if !keep {return VSC_STALE|vsc_work(e);}
        if (e.y&VSC_SHOWN)==0u {
            if VSC_STATS {atomicAdd(&vsc_stats[4],1u);}
            return VSC_UNSHOWN|(e.y&7u);
        }
        return select(0u,vsm.n8e.x,(e.y&1u)!=0u);
    }
    return VSC_MISS;
}

var<workgroup> vsc_wg_misses:atomic<u32>;
var<workgroup> vsc_wg_base:u32;
var<workgroup> vsc_wg_heavy:atomic<u32>;
var<workgroup> vsc_wg_heavy_base:u32;
// Cells whose last walk did at least 2^(VSC_HEAVY-1) work units walk first: the probe queues
// them from the queue's top end down (their count in the word before the cursor), the others
// from word 0 up, and the walk takes the top end first (N8m: the slowest walks must not start
// last; N8n measured the queued walk spending 60% of its time in a tail).
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
            heavy=miss && visible!=VSC_MISS && (visible&7u)>=VSC_HEAVY;
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
// VSC_SPREAD lanes, up to VSC_LANES lanes in all), then its dispatch and the insert passes'.
// N8m's schedule: VSC_PERSIST workgroups (0: one per 32 lanes) take 32-lane chunks from a cursor,
// the queue buffer's last word (after the queue's), which this pass zeroes.
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

// The exact walk for queue lane g (each queued pixel on `spread` lanes), with Exact's setup
// (vsm_prepare: the pixel's receiver point, directions and per-pixel jitter). Word 2k+1 gets bit
// 31, the visible count, the start level << 4 and the work's bit length << 8 for vsc_claim.
fn vsc_walk(g:u32) {
    let spread=max(vsm_scratch.spread,1u);
    if g%spread!=0u {return;}
    let heavy=vsm_scratch.pad0;
    let slot_k=g/spread;
    if slot_k>=atomicLoad(&vsm_scratch.queue)+heavy {return;}
    let k=vsc_entry(slot_k,heavy);
    let word=atomicLoad(&vsm_scratch.words[2u*k]);
    let id=vec2<u32>(word&0xffffu,word>>16u);
    let px=vsm_prepare(id);
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

// ---- Inserting the frame's walks (N8o): claim, then resolve ----
// A queued result's pixel, cell and key, recomputed alike by both passes.
struct VscResult {valid:bool,p:vec2<i32>,cell:VscCell,key:VscKey};
fn vsc_result(entry:u32)->VscResult {
    var out:VscResult;
    let word=atomicLoad(&vsm_scratch.words[2u*entry]);
    out.p=vec2<i32>(vec2<u32>(word&0xffffu,word>>16u));
    let record=textureLoad(vsm_hit,out.p,0);
    let world=textureLoad(vsm_world,out.p,0);
    let normal=textureLoad(vsm_normal,out.p,0);
    out.valid=vsc_shadowed(record,world,normal);
    out.cell=vsc_cell(world.xyz);
    out.key=vsc_key(record.x-1u,vsc_side(world.xyz,normal.xyz,normal.w<0.0),out.cell);
    return out;
}
// A claim's priority: the verdict grade (2 fully lit, 1 fully blocked, both certified by the
// pixel's neighbourhood within margin; 0 neither), the margin exponent, then the pixel (unique).
fn vsc_priority(grade:u32,margin:u32,p:vec2<i32>)->u32 {
    let size=textureDimensions(vsm_world);
    return (grade<<30u)|(margin<<27u)|(u32(p.x)+u32(p.y)*size.x+1u);
}
// The verdict of pixel p's walk with `visible` of k directions open, and its margin exponent:
// grade 2 or 1 when the walk is solid and every receiver within VSC_CONSENSUS pixels finished
// this frame with its count, and the farthest of them is within c.size * 2^margin (margin at
// most 7) of the cell's centre; else grade 0.
fn vsc_verdict(p:vec2<i32>,c:VscCell,visible:u32)->vec2<u32> {
    let k=vsm.n8e.x;
    if visible!=k && visible!=0u {return vec2<u32>(0u);}
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
            if textureLoad(vsc_counts,q,0).x!=count {return vec2<u32>(0u);}
            far=max(far,length(textureLoad(vsm_world,q,0).xyz-centre));
        }
    }
    var margin=0u;
    while margin<8u && c.size*exp2(f32(margin))<far {margin+=1u;}
    if margin==8u {return vec2<u32>(0u);}
    return vec2<u32>(select(1u,2u,visible==k),margin);
}
// The slot a result writes, from the table as this frame's probe left it (vsc_resolve writes
// it only after every claim): its cell's own, else the first slot that is empty or not current
// from one the fingerprint picks, else that one.
fn vsc_target(key:VscKey)->u32 {
    let base=key.bucket*VSC_SLOTS;
    let pick=(key.fingerprint>>8u)&(VSC_SLOTS-1u);
    var free=VSC_SLOTS;
    for(var s=0u;s<VSC_SLOTS;s+=1u) {
        let e=vsc_table[base+s];
        if e.x==key.fingerprint && ((e.y>>8u)&15u)==key.check {return base+s;}
        let t=(pick+s)&(VSC_SLOTS-1u);
        let f=vsc_table[base+t];
        if free==VSC_SLOTS && !vsc_current(f) {free=t;}
    }
    return base+select(free,pick,free==VSC_SLOTS);
}
// Claims each queued result's slot. Word 2k+1 becomes the result with its verdict: bit 31, the
// visible count, start level << 4, work << 8, grade << 12, margin << 14.
@compute @workgroup_size(64) fn vsc_claim(@builtin(global_invocation_id) gid:vec3<u32>) {
    let heavy=vsm_scratch.pad0;
    if gid.x>=atomicLoad(&vsm_scratch.queue)+heavy {return;}
    let k=vsc_entry(gid.x,heavy);
    let result=atomicLoad(&vsm_scratch.words[2u*k+1u]);
    if (result&0x80000000u)==0u {return;}
    let x=vsc_result(k);
    if !x.valid {return;}
    let verdict=vsc_verdict(x.p,x.cell,result&15u);
    let priority=vsc_priority(verdict.x,verdict.y,x.p);
    let slot=vsc_target(x.key);
    atomicMax(&vsc_claims[2u*slot],priority);
    atomicMax(&vsc_claims[2u*slot+1u],~priority);
    atomicStore(&vsm_scratch.words[2u*k+1u],(result&0x80000fffu)|(verdict.x<<12u)|(verdict.y<<14u));
}
// The highest claim on each slot writes it. The cell is shown when every claim on the slot had
// its verdict (all its pixels walked to the same solid count, each certified by its
// neighbourhood), unless the slot already holds it current and not shown: a cell once seen
// beside a differing count stays unshown until it is due again. The winner clears the claims.
@compute @workgroup_size(64) fn vsc_resolve(@builtin(global_invocation_id) gid:vec3<u32>) {
    let heavy=vsm_scratch.pad0;
    if gid.x>=atomicLoad(&vsm_scratch.queue)+heavy {return;}
    let k=vsc_entry(gid.x,heavy);
    let result=atomicLoad(&vsm_scratch.words[2u*k+1u]);
    if (result&0x80000000u)==0u {return;}
    let x=vsc_result(k);
    if !x.valid {return;}
    let grade=(result>>12u)&3u;
    let margin=(result>>14u)&7u;
    let priority=vsc_priority(grade,margin,x.p);
    let base=x.key.bucket*VSC_SLOTS;
    var slot=VSC_SLOTS;
    for(var s=0u;s<VSC_SLOTS;s+=1u) {
        if atomicLoad(&vsc_claims[2u*(base+s)])==priority {slot=base+s;}
    }
    if slot==VSC_SLOTS {return;}
    let lowest=~atomicLoad(&vsc_claims[2u*slot+1u]);
    atomicStore(&vsc_claims[2u*slot],0u);
    atomicStore(&vsc_claims[2u*slot+1u],0u);
    let e=vsc_table[slot];
    let kept=e.x==x.key.fingerprint && ((e.y>>8u)&15u)==x.key.check && vsc_current(e) && (e.y&VSC_SHOWN)==0u;
    let shown=grade!=0u && (lowest>>30u)==grade && !kept;
    let field=select(min((result>>8u)&15u,7u),u32(grade==2u)|(margin<<1u),shown);
    let epochs=((vsc.casters&63u)<<12u)|((vsc.sun&8191u)<<19u);
    vsc_table[slot]=vec2<u32>(x.key.fingerprint,field|(result&0xf0u)|(x.key.check<<8u)|epochs|select(0u,VSC_SHOWN,shown));
}
