// N8/N8b/N8c/N8e: non-RT Interactive sun visibility (docs/non-rt-n8.md, non-rt-n8b.md,
// non-rt-n8c.md, non-rt-n8e.md). No ray
// queries and no acceleration structure: each receiver's 8 Interactive directions get
//   - an exact watertight test against the receiver triangle and its vertex 1-ring, over
//     the whole ray (the existing edge adjacency, walked as vertex fans; skipped where the
//     triangle's precomputed ring horizon is below every direction), and
//   - a shadow-map march (SMRT-style) through a cached sun clipmap from the ray origin up to
//     the highest caster, through the receiver's level and every coarser one, skipping map
//     texels whose top triangle is in that 1-ring.
// Each level keeps two layers, each a surface height with its triangle id: the highest
// surface toward the sun and the highest other surface at the texel centre (a depth peel),
// so an occluder under a higher sheet stays visible. A sample whose layer surface is above
// the ray is decided exactly: a walk from that texel's triangle to the triangle under the
// ray, refined toward the predicted crossing (vsm_top_hits); no texel height blocks a ray.
// Near steps are shared by all eight directions and later steps first test a max-mip of
// the top layer over the cone's footprint (N8c).
// The clipmap is page-driven like a virtual shadow map: receivers request 128-texel
// pages, only requested pages that are stale are rasterized, from the meshes' clusters
// (N8c; optionally a per-level LOD cut), and rasterized pages stay valid until the sun turns, the
// level window moves, or a moved object's light-space bounds touch them.
// N8e (docs/non-rt-n8e.md): a walk cap (proven blocks close a receiver's undecided
// directions), fewer directions per frame, and a rolling refresh that looks up one trace tile
// per block while the frame otherwise holds still (vsm_lookup).
// N8l (docs/non-rt-n8l.md): the lookup's per-receiver direction and shear tables live in
// per-lane workgroup slots (runtime-indexed private arrays went to local memory), and walks
// and ring fans load a triangle's neighbours beside its record, with each batch's adjacency
// base and record range read once per walk or ring.
// N8m (docs/non-rt-n8m.md): persistent lookup workgroups take trace tiles from a queue, heaviest
// first by the work each tile's last lookup recorded (vsm_lookup, vsm_order_*).
// virtual_shadow_gpu.rs prepends PRIMARY_BATCHES and owns every layout below.

const VSM_PAGE:u32=128u;
const VSM_NONE:u32=0xffffffffu;
const VSM_MAX_LEVELS:u32=8u;
const VSM_MAX_RECTS:u32=16u;
const VSM_MISSING:u32=0xffffffffu;
// Ring walk bound per fan direction; valence above this is truncated.
const VSM_FAN_STEPS:u32=24u;

// origin: window minimum corner in eye-relative light-space metres, texel (metres) and the
// caster cut's error bound (metres; negative: original geometry). info: page epoch and
// absolute level (levels[0].info.y).
struct VsmLevel {origin:vec4<f32>,info:vec4<u32>};
struct VsmFrame {
    u:vec4<f32>,v:vec4<f32>,l:vec4<f32>,
    // xyz: world eye; w: pixel footprint per metre of distance (2 tan(fov_y/2)/height).
    eye:vec4<f32>,
    // x: lowest caster height along l, y: 1/height range, z: height range, w: highest.
    // Map heights are normalized to this range.
    depth:vec4<f32>,
    // x: near split r (m), y: finest texel s0 (m), z: disk slope, w: log2 texels per pixel.
    params:vec4<f32>,
    // x: levels, y: pages per window side, z: caster cluster instances, w: invalidation rects.
    counts:vec4<u32>,
    // x: primitive bits of the id map, y: march samples per direction, z: bias in ulps of
    // height range, w: bit 0 to walk the receiver's vertex 1-ring (else its own triangle
    // only), bit 1 to peel under the top layer's own placement too (N8b `peel_own`), bits 2-3
    // the first-step block mode (N8c `first_block`).
    misc:vec4<u32>,
    levels:array<VsmLevel,VSM_MAX_LEVELS>,
    // Eye-relative light-space xy rectangles (min.xy, max.xy) of moved casters, and (N8f)
    // the pixel rectangles of their old and new bounds.
    rects:array<vec4<f32>,VSM_MAX_RECTS>,
    srects:array<vec4<f32>,VSM_MAX_RECTS>,
    // Per traced batch, the first word of its triangles in vsm_horizons (N8b).
    horizon:array<vec4<u32>,PRIMARY_BATCHES/4>,
    // N8h: per traced batch (two per vec4), its first walk record in the draw list's record
    // numbering and its recorded triangles: primitives below the count have records, the
    // others (appended LOD triangles under WalkRecords::Source) read the chains.
    records:array<vec4<u32>,PRIMARY_BATCHES/2>,
    // N8e: x directions evaluated (1-8, the first of the eight); y the rolling refresh (block
    // a in bits 0-3, b in 4-7, this frame's tile in the block in 8-11 and 12-15, bit 16 set on
    // a rolling frame); z the walk cap (0: none); w bit 0 on a rolling frame that moved casters
    // (N8f: every tile is visited, see vsm_reached).
    n8e:vec4<u32>,
    // P-NSQ (docs/p-nsq.md): x the receiver's batches of n8e.x directions this frame (TR1's
    // shadow budget over eight; 1 otherwise), y the frame's rotation into each pixel's
    // direction sequence (TR1 frames; 0 otherwise). Batch b evaluates sequence indices
    // y + 8b + j, so a frame tests RT Interactive's directions [y, y + 8x). z: 1 when a
    // direction one of whose candidate walks ran out of steps is blocked at the march's end
    // (`truncated_walks_block`, vsm_march). w: a candidate walk's step budget at the
    // receiver's level (`walk_steps`; 0: VSM_WALK_STEPS).
    nsq:vec4<u32>,
};
@group(0) @binding(0) var<uniform> vsm:VsmFrame;

fn vsm_light(world:vec3<f32>)->vec3<f32> {
    let rel=world-vsm.eye.xyz;
    return vec3<f32>(dot(rel,vsm.u.xyz),dot(rel,vsm.v.xyz),dot(world,vsm.l.xyz));
}
fn vsm_texel(level:u32)->f32 {return vsm.levels[level].origin.z;}
fn vsm_extent(level:u32)->f32 {return f32(vsm.counts.y*VSM_PAGE)*vsm_texel(level);}
// The light-space height a level's march can serve with its Â±1-page neighbourhood marked:
// the disk's drift over it stays within one page.
fn vsm_cap(level:u32)->f32 {return f32(VSM_PAGE-4u)*vsm_texel(level)/max(vsm.params.z,1e-3);}
fn vsm_reach(level:u32,height:f32)->f32 {return max(0.0,min(vsm_cap(level),vsm.depth.w-height));}
fn vsm_radius(level:u32,height:f32)->f32 {
    return vsm.params.z*vsm_reach(level,height)+3.0*vsm_texel(level);
}
// The finest level at about params.w texels per pixel whose window holds the receiver's
// whole march neighbourhood; coarser levels are the fallback near window edges.
fn vsm_select(world:vec3<f32>)->u32 {
    let p=vsm_light(world);
    let footprint=length(world-vsm.eye.xyz)*vsm.eye.w;
    let k=floor(log2(max(footprint,1e-9)/vsm.params.y)-vsm.params.w);
    let first=u32(clamp(k-f32(vsm.levels[0].info.y),0.0,f32(vsm.counts.x-1u)));
    for(var i=first;i<vsm.counts.x;i+=1u) {
        let r=vsm_radius(i,p.z);
        let lo=vsm.levels[i].origin.xy;let hi=lo+vec2<f32>(vsm_extent(i));
        if all(p.xy-vec2<f32>(r)>=lo) && all(p.xy+vec2<f32>(r)<hi) {return i;}
    }
    return VSM_NONE;
}

// ---- Page requests from the traced receivers ----
@group(0) @binding(1) var<storage,read_write> vsm_requests:array<u32>;
@group(0) @binding(7) var vsm_hit:texture_2d<u32>;
@group(0) @binding(8) var vsm_tiles:texture_2d<u32>;
@group(0) @binding(9) var vsm_world:texture_2d<f32>;
@group(0) @binding(10) var vsm_normal:texture_2d<f32>;
@group(0) @binding(14) var vsm_level_out:texture_storage_2d<r32uint,write>;
// primary_rays.wgsl PRIMARY_TILE.
const VSM_TRACE_TILE=vec2<u32>(8u,4u);

fn vsm_covered(id:vec2<u32>)->bool {
    return textureLoad(vsm_tiles,id/VSM_TRACE_TILE,0).x!=0u && textureLoad(vsm_hit,vec2<i32>(id),0).x!=0u;
}
@compute @workgroup_size(8,4) fn vsm_mark(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(vsm_world);
    if any(id.xy>=size) {return;}
    // Uncovered tiles never had hit records written; read their flag first.
    if textureLoad(vsm_tiles,id.xy/VSM_TRACE_TILE,0).x==0u {return;}
    var level=VSM_NONE;
    if textureLoad(vsm_hit,vec2<i32>(id.xy),0).x!=0u {
        let world=textureLoad(vsm_world,vec2<i32>(id.xy),0).xyz;
        level=vsm_select(world);
        if level!=VSM_NONE {
            let p=vsm_light(world);
            let pages=vsm.counts.y;
            // The march continues through coarser levels until it passes the highest caster
            // (N8b: occluders farther above than a level's reach); each needs its pages.
            for(var l=level;l<vsm.counts.x;l+=1u) {
                let r=vsm_radius(l,p.z);
                let page=f32(VSM_PAGE)*vsm_texel(l);
                let lo=vsm.levels[l].origin.xy;
                let a=vec2<u32>(clamp(floor((p.xy-vec2<f32>(r)-lo)/page),vec2<f32>(0.0),vec2<f32>(f32(pages-1u))));
                let b=vec2<u32>(clamp(floor((p.xy+vec2<f32>(r)-lo)/page),vec2<f32>(0.0),vec2<f32>(f32(pages-1u))));
                for(var y=a.y;y<=b.y;y+=1u) {
                    for(var x=a.x;x<=b.x;x+=1u) {
                        // Benign race: every writer stores the same value.
                        vsm_requests[(l*pages+y)*pages+x]=1u;
                    }
                }
                if vsm_cap(l)>=vsm.depth.w-p.z {break;}
            }
        }
    }
    // The lookup reads this level: one rounding of vsm_select for both passes.
    textureStore(vsm_level_out,vec2<i32>(id.xy),vec4<u32>(level));
}

// ---- Page cache: stale requested pages are flagged for this frame's raster ----
@group(0) @binding(2) var<storage,read_write> vsm_state:array<u32>;
// Per page slot: 1 when the page is cleared and rasterized this frame (zeroed by the
// encoder on page-work frames).
@group(0) @binding(29) var<storage,read_write> vsm_render:array<u32>;

fn vsm_moved(level:u32,page:vec2<u32>)->bool {
    let size=f32(VSM_PAGE)*vsm_texel(level);
    let lo=vsm.levels[level].origin.xy+vec2<f32>(page)*size;let hi=lo+vec2<f32>(size);
    for(var i=0u;i<vsm.counts.w;i+=1u) {
        let r=vsm.rects[i];
        if all(lo<=r.zw) && all(hi>=r.xy) {return true;}
    }
    return false;
}
// N8f: the caster cull's indirect dispatch (x, y, z), zeroed by the encoder and set once a
// page is flagged, so page-work frames that render no page (camera motion) skip the cull.
@group(0) @binding(37) var<storage,read_write> vsm_cull_args:array<atomic<u32>,4>;
@compute @workgroup_size(64) fn vsm_compact(@builtin(global_invocation_id) id:vec3<u32>) {
    let pages=vsm.counts.y;let per=pages*pages;
    let level=id.x/per;
    if level>=vsm.counts.x {return;}
    let local=id.x%per;
    let page=vec2<u32>(local%pages,local/pages);
    let epoch=vsm.levels[level].info.x;
    let moved=vsm_moved(level,page);
    if vsm_requests[id.x]!=0u {
        if vsm_state[id.x]!=epoch || moved {
            vsm_render[id.x]=1u;
            vsm_state[id.x]=epoch;
            // vsm_cull's grid: 64 clusters a group, 65535 groups a row.
            let groups=max((vsm.counts.z+63u)/64u,1u);
            atomicMax(&vsm_cull_args[0],min(groups,65535u));
            atomicMax(&vsm_cull_args[1],(groups+65534u)/65535u);
            atomicMax(&vsm_cull_args[2],1u);
        }
    } else if moved {
        // Not needed now, but its cached depths no longer describe the scene.
        vsm_state[id.x]=0u;
    }
}

// ---- Page raster (N8c): a per-level cluster cut, culled to this frame's stale pages ----
// Casters are the meshes' visibility-buffer clusters (visibility_buffer.rs): original
// geometry, or a clusterLOD hierarchy whose cut at level k keeps every cluster whose
// simplification error is within levels[k].origin.w metres (texel-scale; negative draws
// original geometry only). The cut depends on the sun's clipmap alone, so camera motion
// keeps cached pages valid. Small clusters are rasterized in software, one workgroup per
// cluster; large ones in hardware. Both write max(height << 32 | id) per texel centre into
// 64-bit maps, top layer first, then the peel layer: the highest surface other than the
// texel's top triangle. Writes are confined to the pages flagged in vsm_render.
struct VsmCluster {lo:vec3<f32>,vertices:u32,hi:vec3<f32>,triangles:u32,owner:u32,refined:u32,primitives:u32,counts:u32,placement:u32,pad0:u32,pad1:u32,pad2:u32};
struct VsmGroup {sphere:vec4<f32>,error:f32,pad0:u32,pad1:u32,pad2:u32};
@group(0) @binding(25) var<storage,read> vsm_clusters:array<VsmCluster>;
@group(0) @binding(26) var<storage,read> vsm_groups:array<VsmGroup>;
// Per level: software dispatch (x, y, z, count) and hardware draw (vertices, instances,
// first vertex, first instance), zeroed by the encoder before the cull; then per level
// software cluster slots and hardware ones (counts.z each).
struct VsmArgs {sw_x:u32,sw_y:u32,sw_z:u32,sw_count:atomic<u32>,hw_vertices:u32,hw_instances:atomic<u32>,hw_first:u32,hw_first_instance:u32};
struct VsmWork {args:array<VsmArgs,VSM_MAX_LEVELS>,lists:array<u32>};
@group(0) @binding(27) var<storage,read_write> vsm_work:VsmWork;
// A read-only view for the raster, which also uses the arguments as its indirect buffer.
@group(0) @binding(33) var<storage,read> vsm_work_in:array<u32>;
const VSM_LISTS:u32=8u*VSM_MAX_LEVELS;
@group(0) @binding(31) var<storage,read_write> vsm_top_map:array<atomic<u64>>;
@group(0) @binding(32) var<storage,read_write> vsm_peel_map:array<atomic<u64>>;
// Its own group: wgpu forbids binding arrays beside uniform buffers.
@group(2) @binding(8) var<storage,read> vsm_cluster_data:binding_array<VsmU32s,PRIMARY_BATCHES>;
struct VsmPass {level:u32,pad0:u32,pad1:u32,pad2:u32};
@group(3) @binding(30) var<uniform> vsm_pass:VsmPass;
// Clusters whose light-space box spans more texels than this use the hardware raster.
override VSM_SW_EXTENT:f32=32.0;
// The peel-layer variant of the raster entry points.
override VSM_PEEL_PASS:bool=false;

// World-space simplification error of group g under placement's conservative affine
// scale (visibility_buffer_cull.wgsl's group_error without the projection).
fn vsm_group_error(g:u32,placement:u32)->f32 {
    let s=vsm_groups[g];
    if s.error>=3.0e38 {return 3.0e38;}
    let m=models[placement];
    let a=abs(m[0].xyz);let b=abs(m[1].xyz);let c=abs(m[2].xyz);
    let columns=max(a.x+a.y+a.z,max(b.x+b.y+b.z,c.x+c.y+c.z));
    let rows=max(a.x+b.x+c.x,max(a.y+b.y+c.y,a.z+b.z+c.z));
    return s.error*sqrt(columns*rows);
}
@compute @workgroup_size(64) fn vsm_cull(@builtin(global_invocation_id) id:vec3<u32>) {
    let index=id.x+id.y*65535u*64u;
    if index>=vsm.counts.z {return;}
    let c=vsm_clusters[index];
    let m=models[c.placement];
    var lo=vec2<f32>(3.4e38);var hi=vec2<f32>(-3.4e38);
    for(var i=0u;i<8u;i+=1u) {
        let p=vec3<f32>(select(c.lo.x,c.hi.x,(i&1u)!=0u),select(c.lo.y,c.hi.y,(i&2u)!=0u),select(c.lo.z,c.hi.z,(i&4u)!=0u));
        let q=vsm_light((m*vec4<f32>(p,1.0)).xyz).xy;
        lo=min(lo,q);hi=max(hi,q);
    }
    let coarse=vsm_group_error(c.owner,c.placement);
    let own=select(vsm_group_error(c.refined,c.placement),-3.4e38,c.refined==0xffffffffu);
    let pages=vsm.counts.y;
    for(var level=0u;level<vsm.counts.x;level+=1u) {
        // clusterlod's cut: the coarser group is too coarse, this cluster fine enough.
        let bound=vsm.levels[level].origin.w;
        if !(coarse>bound && own<=bound) {continue;}
        let texel=vsm_texel(level);
        let page=f32(VSM_PAGE)*texel;
        let origin=vsm.levels[level].origin.xy;
        // One texel of padding covers the rounding between box and raster transforms.
        let a=floor((lo-vec2<f32>(texel)-origin)/page);
        let b=floor((hi+vec2<f32>(texel)-origin)/page);
        if any(b<vec2<f32>(0.0)) || any(a>=vec2<f32>(f32(pages))) {continue;}
        let p0=vec2<u32>(max(a,vec2<f32>(0.0)));
        let p1=vec2<u32>(min(b,vec2<f32>(f32(pages-1u))));
        var stale=false;
        for(var y=p0.y;y<=p1.y && !stale;y+=1u) {
            for(var x=p0.x;x<=p1.x;x+=1u) {
                if vsm_render[(level*pages+y)*pages+x]!=0u {stale=true;break;}
            }
        }
        if !stale {continue;}
        let extent=(hi-lo)/texel;
        let n=vsm.counts.z;
        if max(extent.x,extent.y)>VSM_SW_EXTENT {
            let slot=atomicAdd(&vsm_work.args[level].hw_instances,1u);
            vsm_work.lists[(2u*level+1u)*n+slot]=index;
        } else {
            let slot=atomicAdd(&vsm_work.args[level].sw_count,1u);
            vsm_work.lists[2u*level*n+slot]=index;
        }
    }
}
@compute @workgroup_size(8) fn vsm_finish(@builtin(local_invocation_index) level:u32) {
    let n=atomicLoad(&vsm_work.args[level].sw_count);
    vsm_work.args[level].sw_x=min(n,65535u);
    vsm_work.args[level].sw_y=select(0u,(n+65534u)/65535u,n!=0u);
    vsm_work.args[level].sw_z=1u;
    vsm_work.args[level].hw_vertices=384u;
    vsm_work.args[level].hw_first=0u;
    vsm_work.args[level].hw_first_instance=0u;
}
// Zeroes both layers of every page flagged for this frame: one workgroup per page slot.
@compute @workgroup_size(256) fn vsm_clear(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    let slot=group.x;
    if vsm_render[slot]==0u {return;}
    let pages=vsm.counts.y;let per=pages*pages;
    let level=slot/per;let local=slot%per;
    let side=pages*VSM_PAGE;
    let corner=vec2<u32>(local%pages,local/pages)*VSM_PAGE;
    for(var k=li;k<VSM_PAGE*VSM_PAGE;k+=256u) {
        let t=corner+vec2<u32>(k%VSM_PAGE,k/VSM_PAGE);
        let i=(level*side+t.y)*side+t.x;
        atomicStore(&vsm_top_map[i],0lu);
        atomicStore(&vsm_peel_map[i],0lu);
    }
}
// The map word of one caster fragment at texel t of `level` (normalized height in [0,1]).
fn vsm_write(level:u32,t:vec2<u32>,height:f32,id:u32) {
    let pages=vsm.counts.y;
    let page=t/VSM_PAGE;
    if vsm_render[(level*pages+page.y)*pages+page.x]==0u {return;}
    let side=pages*VSM_PAGE;
    let i=(level*side+t.y)*side+t.x;
    let value=(u64(bitcast<u32>(clamp(height,0.0,1.0)))<<32u)|u64(id);
    if VSM_PEEL_PASS {
        // Only one triangle of a watertight surface covers a texel centre, so this is the
        // next surface down.
        if u32(atomicLoad(&vsm_top_map[i])&0xfffffffflu)==id {return;}
        atomicMax(&vsm_peel_map[i],value);
    } else {
        atomicMax(&vsm_top_map[i],value);
    }
}
// Placement + 1 and primitive, so the lookup can recognize the receiver's own 1-ring.
fn vsm_cluster_id(c:VsmCluster,batch:u32,local:u32)->u32 {
    var primitive=c.primitives+local;
    if (c.counts&0x10000u)!=0u {primitive=vsm_cluster_data[batch].data[c.primitives+local];}
    return ((c.placement+1u)<<vsm.misc.x)|primitive;
}
// A cluster vertex in level texels (x, y) and normalized height.
fn vsm_cluster_vertex(c:VsmCluster,batch:u32,local:u32,level:u32)->vec3<f32> {
    let o=c.vertices+3u*local;
    let d=&vsm_cluster_data[batch].data;
    let p=vec3<f32>(bitcast<f32>((*d)[o]),bitcast<f32>((*d)[o+1u]),bitcast<f32>((*d)[o+2u]));
    let q=vsm_light((models[c.placement]*vec4<f32>(p,1.0)).xyz);
    let texel=vsm_texel(level);
    return vec3<f32>((q.xy-vsm.levels[level].origin.xy)/texel,(q.z-vsm.depth.x)*vsm.depth.y);
}
var<workgroup> vsm_wg_texels:array<vec3<f32>,128>;
var<workgroup> vsm_wg_cluster:u32;
fn vsm_edge(a:vec2<i32>,b:vec2<i32>,p:vec2<i32>)->i32 {return (b.x-a.x)*(p.y-a.y)-(b.y-a.y)*(p.x-a.x);}
// Software raster: the cluster's vertices are transformed once into workgroup memory, then
// one invocation per triangle samples the texel centres it covers. Vertices snap to 8
// subpixel bits with exact integer edge functions; edges are inclusive (the max makes double
// coverage harmless) and both windings are kept.
@compute @workgroup_size(128) fn vsm_raster(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    let level=vsm_pass.level;
    let slot=group.x+group.y*65535u;
    let n=vsm.counts.z;
    if li==0u {vsm_wg_cluster=select(VSM_NONE,vsm_work_in[VSM_LISTS+2u*level*n+slot],slot<vsm_work_in[8u*level+3u]);}
    let index=workgroupUniformLoad(&vsm_wg_cluster);
    if index==VSM_NONE {return;}
    let c=vsm_clusters[index];
    let batch=primary_placements[c.placement]&0x1fffffffu;
    if li<((c.counts>>8u)&255u) {vsm_wg_texels[li]=vsm_cluster_vertex(c,batch,li,level);}
    workgroupBarrier();
    if li>=(c.counts&255u) {return;}
    let t=vsm_cluster_data[batch].data[c.triangles+li];
    let s0=vsm_wg_texels[t&255u];var s1=vsm_wg_texels[(t>>8u)&255u];var s2=vsm_wg_texels[(t>>16u)&255u];
    let a=vec2<i32>(round(s0.xy*256.0));var b=vec2<i32>(round(s1.xy*256.0));var d=vec2<i32>(round(s2.xy*256.0));
    let area=vsm_edge(a,b,d);
    if area==0 {return;}
    if area<0 {let p=b;b=d;d=p;let z=s1;s1=s2;s2=z;}
    let inverse=1.0/f32(abs(area));
    let side=i32(vsm.counts.y*VSM_PAGE);
    // Texel centres are at 256x+128 subpixels.
    let lo=max(min(a,min(b,d)),vec2<i32>(0));
    let hi=min(max(a,max(b,d)),vec2<i32>(side*256-1));
    let first=(lo-vec2<i32>(128)+vec2<i32>(255))>>vec2<u32>(8u);
    let last=(hi-vec2<i32>(128))>>vec2<u32>(8u);
    let id=vsm_cluster_id(c,batch,li);
    for(var y=first.y;y<=last.y;y+=1) {
        for(var x=first.x;x<=last.x;x+=1) {
            let p=vec2<i32>(x*256+128,y*256+128);
            let w0=vsm_edge(b,d,p);let w1=vsm_edge(d,a,p);let w2=vsm_edge(a,b,p);
            if (w0|w1|w2)<0 {continue;}
            let z=(f32(w0)*s0.z+f32(w1)*s1.z+f32(w2)*s2.z)*inverse;
            vsm_write(level,vec2<u32>(u32(x),u32(y)),z,id);
        }
    }
}
// Hardware raster of large clusters: one instance per cluster, 128 triangle slots; unused
// slots collapse to a point. The attachment only sizes the raster area.
struct VsmHw {@builtin(position) position:vec4<f32>,@location(0) @interpolate(flat) id:u32,@location(1) height:f32};
@vertex fn vs_vsm_cluster(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->VsmHw {
    let level=vsm_pass.level;
    let index=vsm_work_in[VSM_LISTS+(2u*level+1u)*vsm.counts.z+instance];
    let c=vsm_clusters[index];let local=vertex/3u;
    var out:VsmHw;
    out.position=vec4<f32>(0.0,0.0,0.0,1.0);
    if local<(c.counts&255u) {
        let batch=primary_placements[c.placement]&0x1fffffffu;
        let t=vsm_cluster_data[batch].data[c.triangles+local];
        let s=vsm_cluster_vertex(c,batch,(t>>(8u*(vertex%3u)))&255u,level);
        let side=f32(vsm.counts.y*VSM_PAGE);
        out.position=vec4<f32>(s.x/side*2.0-1.0,1.0-s.y/side*2.0,0.5,1.0);
        out.height=s.z;
        out.id=vsm_cluster_id(c,batch,local);
    }
    return out;
}
@fragment fn fs_vsm_cluster(input:VsmHw) {
    vsm_write(vsm_pass.level,vec2<u32>(input.position.xy),input.height,input.id);
}

// ---- Lookup: exact 1-ring, then the map march beyond `near` ----
@group(0) @binding(15) var vsm_output:texture_storage_2d<r32uint,write>;
// The 64-bit maps as (id, normalized height bits) words; empty texels are zero.
@group(0) @binding(16) var<storage,read> vsm_top_words:array<vec2<u32>>;
@group(0) @binding(18) var vsm_level_in:texture_2d<u32>;
@group(0) @binding(19) var<storage,read> vsm_peel_words:array<vec2<u32>>;
fn vsm_word(cell:vec2<i32>,level:u32)->u32 {
    let side=vsm.counts.y*VSM_PAGE;
    return (level*side+u32(cell.y))*side+u32(cell.x);
}

struct NormalTransform {x:vec4<f32>,y:vec4<f32>,z:vec4<f32>,info:vec4<f32>};
struct VsmVec4s {data:array<vec4<f32>>};
struct VsmU32s {data:array<u32>};
@group(1) @binding(0) var<storage,read> models:array<mat4x4<f32>>;
@group(1) @binding(2) var<storage,read> primary_vertices:binding_array<VsmVec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> primary_indices:binding_array<VsmU32s,PRIMARY_BATCHES>;
@group(1) @binding(5) var<storage,read> primary_placements:array<u32>;

struct LightingUniform {eye:vec3<f32>,count:u32,environment:vec3<f32>,exposure:f32,map:vec4<f32>};
struct Light {position_range:vec4<f32>,travel_outer:vec4<f32>,rgb_inner:vec4<f32>,kind:vec4<u32>, source:vec4<f32>};
struct ShadowView {world_to_clip:mat4x4<f32>,coverage_bias:vec4<f32>,camera_depth:vec4<f32>,texel:vec4<f32>};
@group(3) @binding(0) var<uniform> lighting:LightingUniform;
@group(3) @binding(1) var<storage,read> lights:array<Light>;
@group(3) @binding(5) var<storage,read> shadow_views:array<ShadowView>;

fn vsm_triangle(batch:u32,primitive:u32)->array<vec3<f32>,3> {
    let base=primitive*3u;
    return array<vec3<f32>,3>(primary_vertices[batch].data[primary_indices[batch].data[base]*3u].xyz,
        primary_vertices[batch].data[primary_indices[batch].data[base+1u]*3u].xyz,
        primary_vertices[batch].data[primary_indices[batch].data[base+2u]*3u].xyz);
}
// The adjacency suffix of the index buffer (receiver_adjacency.rs): 6T+1 words.
fn vsm_neighbour(batch:u32,primitive:u32,edge:u32)->u32 {
    let words=arrayLength(&primary_indices[batch].data);
    if words%3u!=1u {return VSM_MISSING;}
    return primary_indices[batch].data[(words-1u)/2u+primitive*3u+edge];
}
// N8g walk records (virtual_shadow_records.wgsl, docs/non-rt-n8g.md), N8h's layout
// (docs/non-rt-n8h.md): per recorded triangle its three positions' bits, 36 contiguous bytes,
// in segments of 2^21 records (one binding each) packed across batches. Edge neighbours stay
// in the index buffer's adjacency suffix: a neighbour's address depends only on the primitive,
// so its load issues beside the record's. The lookup reads geometry only through vsm_tri or
// vsm_tri_in and vsm_nb_at or vsm_nbs; a primitive at or above its batch's recorded count, or
// VSM_RECORDS off (screens), reads the chains.
struct VsmRecords {data:array<u32>};
@group(2) @binding(9) var<storage,read> vsm_records:binding_array<VsmRecords,PRIMARY_BATCHES>;
override VSM_RECORDS:bool=true;
const VSM_SEGMENT_BITS:u32=21u;
// The record's segment and first word, or x PRIMARY_BATCHES when the triangle has none.
fn vsm_record_at(batch:u32,primitive:u32)->vec2<u32> {
    let pair=vsm.records[batch/2u];
    let odd=(batch&1u)!=0u;
    if !VSM_RECORDS || primitive>=select(pair.y,pair.w,odd) {return vec2<u32>(PRIMARY_BATCHES,0u);}
    let at=select(pair.x,pair.z,odd)+primitive;
    return vec2<u32>(at>>VSM_SEGMENT_BITS,(at&((1u<<VSM_SEGMENT_BITS)-1u))*9u);
}
fn vsm_tri(batch:u32,primitive:u32)->array<vec3<f32>,3> {
    let at=vsm_record_at(batch,primitive);
    if at.x>=PRIMARY_BATCHES {return vsm_triangle(batch,primitive);}
    let s=at.x;let i=at.y;
    return array<vec3<f32>,3>(
        bitcast<vec3<f32>>(vec3<u32>(vsm_records[s].data[i],vsm_records[s].data[i+1u],vsm_records[s].data[i+2u])),
        bitcast<vec3<f32>>(vec3<u32>(vsm_records[s].data[i+3u],vsm_records[s].data[i+4u],vsm_records[s].data[i+5u])),
        bitcast<vec3<f32>>(vec3<u32>(vsm_records[s].data[i+6u],vsm_records[s].data[i+7u],vsm_records[s].data[i+8u])));
}
// N8l: a batch's (first walk record, recorded count), read once per walk or ring, and a
// triangle's positions from it (vsm_tri's lookup with the range already read).
fn vsm_records_of(batch:u32)->vec2<u32> {
    let pair=vsm.records[batch/2u];
    let odd=(batch&1u)!=0u;
    return vec2<u32>(select(pair.x,pair.z,odd),select(pair.y,pair.w,odd));
}
fn vsm_tri_in(rec:vec2<u32>,batch:u32,primitive:u32)->array<vec3<f32>,3> {
    if !VSM_RECORDS || primitive>=rec.y {return vsm_triangle(batch,primitive);}
    let at=rec.x+primitive;
    let sg=at>>VSM_SEGMENT_BITS;let i=(at&((1u<<VSM_SEGMENT_BITS)-1u))*9u;
    return array<vec3<f32>,3>(
        bitcast<vec3<f32>>(vec3<u32>(vsm_records[sg].data[i],vsm_records[sg].data[i+1u],vsm_records[sg].data[i+2u])),
        bitcast<vec3<f32>>(vec3<u32>(vsm_records[sg].data[i+3u],vsm_records[sg].data[i+4u],vsm_records[sg].data[i+5u])),
        bitcast<vec3<f32>>(vec3<u32>(vsm_records[sg].data[i+6u],vsm_records[sg].data[i+7u],vsm_records[sg].data[i+8u])));
}
// N8l: the first word of a batch's adjacency suffix (VSM_NONE without one), found once per walk
// or ring rather than per neighbour, and neighbours read from it.
fn vsm_adjacency(batch:u32)->u32 {
    let words=arrayLength(&primary_indices[batch].data);
    return select(VSM_NONE,(words-1u)/2u,words%3u==1u);
}
fn vsm_nb_at(batch:u32,adj:u32,primitive:u32,edge:u32)->u32 {
    if adj==VSM_NONE {return VSM_MISSING;}
    return primary_indices[batch].data[adj+primitive*3u+edge];
}
// A triangle's three neighbours, loaded together (beside its record, so a walk or fan step
// picks its exit by select rather than a second dependent load), and one of them.
fn vsm_nbs(batch:u32,adj:u32,primitive:u32)->vec3<u32> {
    if adj==VSM_NONE {return vec3<u32>(VSM_MISSING);}
    let k=adj+primitive*3u;
    return vec3<u32>(primary_indices[batch].data[k],primary_indices[batch].data[k+1u],primary_indices[batch].data[k+2u]);
}
fn vsm_pick(n:vec3<u32>,e:u32)->u32 {return select(select(n.z,n.y,e==1u),n.x,e==0u);}
fn vsm_unit(n:vec3<f32>)->vec3<f32> {
    let scale=max(max(abs(n.x),abs(n.y)),abs(n.z));
    let scaled=n/max(scale,1e-30);
    return scaled*inverseSqrt(max(dot(scaled,scaled),1e-30));
}
// fresh_shadows.wgsl's per-pixel R2 sequence, so N8 traces Interactive's directions.
fn vsm_hash(input:u32)->u32 {
    var x=input;x=(x^(x>>16u))*0x7feb352du;x=(x^(x>>15u))*0x846ca68bu;return x^(x>>16u);
}
fn vsm_direction(index:u32,shift:vec2<f32>,l:vec3<f32>,u:vec3<f32>,v:vec3<f32>,slope:f32)->vec3<f32> {
    let r=fract(vec2<f32>(f32(index))*vec2<f32>(0.7548776662466927,0.5698402909980532)+shift);
    let a=6.283185307179586*r.y;
    return normalize(l+slope*sqrt(r.x)*(cos(a)*u+sin(a)*v));
}

// The watertight ray/triangle test of the N0 software backend (Woop, Benthin and Wald
// 2013), unchanged: shared edges give opposite-signed or zero edge functions.
struct VsmRay {origin:vec3<f32>,direction:vec3<f32>,shear:vec3<f32>,axes:vec3<u32>};
fn vsm_ray(origin:vec3<f32>,direction:vec3<f32>)->VsmRay {
    let a=abs(direction);
    var z=2u;
    if a.x>=a.y && a.x>=a.z {z=0u;} else if a.y>=a.z {z=1u;}
    var x=(z+1u)%3u;var y=(z+2u)%3u;
    if direction[z]<0.0 {let s=x;x=y;y=s;}
    let dz=direction[z];
    return VsmRay(origin,direction,vec3<f32>(direction[x]/dz,direction[y]/dz,1.0/dz),vec3<u32>(x,y,z));
}
fn vsm_difference(a:f32,b:f32,c:f32,d:f32)->f32 {
    let w=c*d;
    return fma(a,b,-w)+fma(-c,d,w);
}
fn vsm_hits(ray:VsmRay,p0:vec3<f32>,p1:vec3<f32>,p2:vec3<f32>,tmin:f32)->bool {
    let k=ray.axes;let s=ray.shear;
    let a=p0-ray.origin;let b=p1-ray.origin;let c=p2-ray.origin;
    let ax=fma(-s.x,a[k.z],a[k.x]);let ay=fma(-s.y,a[k.z],a[k.y]);
    let bx=fma(-s.x,b[k.z],b[k.x]);let by=fma(-s.y,b[k.z],b[k.y]);
    let cx=fma(-s.x,c[k.z],c[k.x]);let cy=fma(-s.y,c[k.z],c[k.y]);
    let u=vsm_difference(cx,by,cy,bx);
    let v=vsm_difference(ax,cy,ay,cx);
    let w=vsm_difference(bx,ay,by,ax);
    if (u<0.0 || v<0.0 || w<0.0) && (u>0.0 || v>0.0 || w>0.0) {return false;}
    let det=u+v+w;
    if det==0.0 {return false;}
    let t=fma(u,s.z*a[k.z],fma(v,s.z*b[k.z],w*(s.z*c[k.z])))/det;
    return t>=tmin;
}

struct VsmReceiver {world:vec3<f32>,geometric:vec3<f32>,epsilon:f32,thin:bool,slot:u32,batch:u32,primitive:u32};
// shadow_origin (scene.wgsl): opaque receivers stay on the viewed side.
fn vsm_origin(r:VsmReceiver,toward_light:vec3<f32>)->vec3<f32> {
    let toward=select(lighting.eye-r.world,toward_light,r.thin);
    let normal=select(-r.geometric,r.geometric,dot(r.geometric,toward)>=0.0);
    return fma(normal,vec3<f32>(r.epsilon),r.world);
}
fn vsm_to_world(v:vec3<f32>,slot:u32)->vec3<f32> {return (models[slot]*vec4<f32>(v,1.0)).xyz;}
fn vsm_same(a:vec3<f32>,b:vec3<f32>)->bool {return all(a==b);}

// Per-pixel lookup state, set once by vsm_lookup: the eight directions, the receiver
// triangle's object-space positions and the opaque receivers' shared origin. Private
// storage avoids copying arrays into every call. N8l: the directions (and the shears below),
// read by runtime direction indices, are per-lane workgroup slots ([component][lane], so a
// warp's reads are conflict-free) rather than private arrays, which the driver had placed in
// local memory.
var<workgroup> vsm_wg_dirs:array<array<f32,32>,24>;
fn vsm_dir(j:u32)->vec3<f32> {return vec3<f32>(vsm_wg_dirs[3u*j][vsm_lane],vsm_wg_dirs[3u*j+1u][vsm_lane],vsm_wg_dirs[3u*j+2u][vsm_lane]);}
fn vsm_dir_store(j:u32,d:vec3<f32>) {vsm_wg_dirs[3u*j][vsm_lane]=d.x;vsm_wg_dirs[3u*j+1u][vsm_lane]=d.y;vsm_wg_dirs[3u*j+2u][vsm_lane]=d.z;}
var<private> vsm_own:array<vec3<f32>,3>;
var<private> vsm_shared_origin:vec3<f32>;
// N8l: this lane's slot in the workgroup tables.
var<private> vsm_lane:u32;
// One axis permutation for all eight directions: the sun's dominant axis (directions are
// within 0.1 rad of it), with x/y swapped when it points down so edge signs keep their
// meaning. Each direction keeps its own shear. Watertightness needs only consistency
// between triangles for one ray, which a fixed permutation keeps.
var<private> vsm_axes:vec3<u32>;
var<workgroup> vsm_wg_shear:array<array<f32,32>,24>;
fn vsm_shear_at(j:u32)->vec3<f32> {return vec3<f32>(vsm_wg_shear[3u*j][vsm_lane],vsm_wg_shear[3u*j+1u][vsm_lane],vsm_wg_shear[3u*j+2u][vsm_lane]);}
// The central direction's shear, and the sheared-plane radius per unit height of the cone
// holding all eight directions.
var<private> vsm_centre:vec3<f32>;
var<private> vsm_cone:f32;
// The pixel's direction sequence (vsm_prepare): its R2 shift, the light basis and disk slope.
var<private> vsm_seq_shift:vec2<f32>;
var<private> vsm_seq_l:vec3<f32>;
var<private> vsm_seq_u:vec3<f32>;
var<private> vsm_seq_v:vec3<f32>;
var<private> vsm_seq_slope:f32;
// N8e: the directions this frame evaluates (bit j: vsm_dir(j)); the others start blocked
// and are not counted. Walk triangles tested so far (the walk cap).
var<private> vsm_active:u32;
var<private> vsm_visits:u32;
// P-NSQ: directions a candidate walk could not decide this march (it ran out of steps before
// locating the triangle under its aim; vsm_top_hits bit 10).
var<private> vsm_undecided:u32;
// N8m: ring triangles tested, the receiver's work besides its walk visits (the tile order's cost).
var<private> vsm_ring_tests:u32;
// Diagnostics (VSM_DEBUG only): the trace scripts/n8-diagnose.py classifies, two uvec4s per
// pixel. Whole-vector read-modify-writes of named components only.
//   x: ring-blocked directions (0-7) | map-blocked (8-15) | level (24-31)
//   y: triangles visited by shared walks (0-15) | by per-direction walks (16-31)
//   z: shared candidates decided (0-15) | per-direction candidates (16-31)
//   w: ring triangles tested (0-15) | fan stopped by: step cap (16), open edge (17),
//      non-manifold corner (18)
// The second word is reserved (zero).
override VSM_DEBUG:bool=false;
// N8b: decide rays exactly below an interior surface beyond the tested neighbourhood.
override VSM_CONTINUE:bool=true;
// March samples in each level beyond the receiver's (N8b continuation).
override VSM_EXTRA_SAMPLES:u32=2u;
// N8b screens: the peel layer in the march, and the march beyond the receiver's level.
override VSM_PEEL:bool=true;
override VSM_ASCEND:bool=true;
// Cost screens: the far march at all, and its first step from the ray origin (off: from
// `near`, as N8 did).
override VSM_FAR:bool=true;
override VSM_ORIGIN_STEP:bool=true;
// The first step's 2x2 texel block around the ray (VirtualShadowSettings::first_block, misc.w
// bits 2-3): near-contact occluder edges that miss the texel centre (N8b). 1: every
// placement (N8b); 2: beyond the texel under the ray, only other placements' surfaces; 0: off.
fn vsm_block_mode()->u32 {return (vsm.misc.w>>2u)&3u;}
// The lookup's scratch (N8m): `queue`, the persistent workgroups' tile queue, then `words`. The
// encoder zeroes the queue and the first VSM_HEAD words before each lookup: the tile order's
// per-bucket counts, then its cursors. After them, per trace tile (vsm_tile_count) its place in
// the order, then the work the last lookup of that tile recorded; then the VSM_DEBUG trace, two
// uvec4s per pixel. (N8k's hybrid binds its receiver queue here instead: the count in `queue`,
// its lanes per receiver in `spread`, two words per receiver in `words`;
// virtual_shadow_hybrid.wgsl.)
const VSM_BUCKETS:u32=16u;
const VSM_HEAD:u32=2u*VSM_BUCKETS;
struct VsmScratch {queue:atomic<u32>,spread:u32,pad0:u32,pad1:u32,words:array<atomic<u32>>};
@group(0) @binding(20) var<storage,read_write> vsm_scratch:VsmScratch;
// Trace tiles in the image: the scratch's per-tile word count.
fn vsm_tile_grid()->vec2<u32> {return (textureDimensions(vsm_world)+VSM_TRACE_TILE-vec2<u32>(1u))/VSM_TRACE_TILE;}
fn vsm_tile_count()->u32 {let g=vsm_tile_grid();return g.x*g.y;}
var<private> vsm_dbg:vec4<u32>;
fn vsm_permute(v:vec3<f32>)->vec3<f32> {return vec3<f32>(v[vsm_axes.x],v[vsm_axes.y],v[vsm_axes.z]);}
fn vsm_prepare_rays(l:vec3<f32>) {
    let a=abs(l);
    var z=2u;
    if a.x>=a.y && a.x>=a.z {z=0u;} else if a.y>=a.z {z=1u;}
    var x=(z+1u)%3u;var y=(z+2u)%3u;
    if l[z]<0.0 {let t=x;x=y;y=t;}
    vsm_axes=vec3<u32>(x,y,z);
    let c=vsm_permute(l);
    vsm_centre=vec3<f32>(c.x/c.z,c.y/c.z,1.0/c.z);
    var cone=0.0;
    for(var j=0u;j<8u;j+=1u) {
        let d=vsm_permute(vsm_dir(j));
        let shear=vec3<f32>(d.x/d.z,d.y/d.z,1.0/d.z);
        if (vsm_active&(1u<<j))!=0u {cone=max(cone,length(shear.xy-vsm_centre.xy));}
        vsm_wg_shear[3u*j][vsm_lane]=shear.x;vsm_wg_shear[3u*j+1u][vsm_lane]=shear.y;vsm_wg_shear[3u*j+2u][vsm_lane]=shear.z;
    }
    vsm_cone=cone/abs(vsm_centre.z);
}
// The watertight test on a triangle already permuted and made relative to the origin.
fn vsm_hits_sheared(s:vec3<f32>,a:vec3<f32>,b:vec3<f32>,c:vec3<f32>,tmin:f32)->bool {
    let ax=fma(-s.x,a.z,a.x);let ay=fma(-s.y,a.z,a.y);
    let bx=fma(-s.x,b.z,b.x);let by=fma(-s.y,b.z,b.y);
    let cx=fma(-s.x,c.z,c.x);let cy=fma(-s.y,c.z,c.y);
    let u=vsm_difference(cx,by,cy,bx);
    let v=vsm_difference(ax,cy,ay,cx);
    let w=vsm_difference(bx,ay,by,ax);
    if (u<0.0 || v<0.0 || w<0.0) && (u>0.0 || v>0.0 || w>0.0) {return false;}
    let det=u+v+w;
    if det==0.0 {return false;}
    let t=fma(u,s.z*a.z,fma(v,s.z*b.z,w*(s.z*c.z)))/det;
    return t>=tmin;
}
fn vsm_ray_origin(r:VsmReceiver,j:u32)->vec3<f32> {
    if r.thin {return vsm_origin(r,vsm_dir(j));}
    return vsm_shared_origin;
}
// Bit j set: direction j is blocked by this world-space triangle.
// Also grows the ring's squared reach: the sheared projection along the sun overestimates
// light-space distance, so it is a safe bound for the map's ring filter.
fn vsm_test(r:VsmReceiver,a:vec3<f32>,b:vec3<f32>,c:vec3<f32>,ring:VsmRing)->VsmRing {
    var result=ring;
    let o=vsm_shared_origin;
    let pa=vsm_permute(a-o);let pb=vsm_permute(b-o);let pc=vsm_permute(c-o);
    let s0=vsm_centre;
    let qa=vec2<f32>(fma(-s0.x,pa.z,pa.x),fma(-s0.y,pa.z,pa.y));
    let qb=vec2<f32>(fma(-s0.x,pb.z,pb.x),fma(-s0.y,pb.z,pb.y));
    let qc=vec2<f32>(fma(-s0.x,pc.z,pc.x),fma(-s0.y,pc.z,pc.y));
    result.radius=max(result.radius,max(dot(qa,qa),max(dot(qb,qb),dot(qc,qc))));
    // Cone cull: the rays rise to height h = t (sheared z), within vsm_cone*h of the central
    // ray. Thin origins differ by up to 2 epsilon; the pad also covers rounding.
    let h=max(pa.z*s0.z,max(pb.z*s0.z,pc.z*s0.z));
    let pad=4.0*r.epsilon+1e-5*max(max(length(pa),length(pb)),length(pc));
    if h<r.epsilon-pad {return result;}
    let gap=max(max(min(qa,min(qb,qc)),-max(qa,max(qb,qc))),vec2<f32>(0.0));
    let reach=vsm_cone*max(h,0.0)+pad;
    if dot(gap,gap)>reach*reach {return result;}
    for(var j=0u;j<8u;j+=1u) {
        if (result.blocked&(1u<<j))!=0u {continue;}
        var hit=false;
        if r.thin {
            let oj=vsm_ray_origin(r,j);
            hit=vsm_hits_sheared(vsm_shear_at(j),vsm_permute(a-oj),vsm_permute(b-oj),vsm_permute(c-oj),r.epsilon);
        } else {
            hit=vsm_hits_sheared(vsm_shear_at(j),pa,pb,pc,r.epsilon);
        }
        if hit {result.blocked|=1u<<j;}
    }
    return result;
}
// The ring's blocked directions and its light-space reach around the receiver.
struct VsmRing {blocked:u32,radius:f32};
// Squared light-space reach; vsm_ring returns its square root.
fn vsm_reach_of(radius:f32,centre:vec2<f32>,a:vec3<f32>,b:vec3<f32>,c:vec3<f32>)->f32 {
    let pa=vsm_light(a).xy-centre;let pb=vsm_light(b).xy-centre;let pc=vsm_light(c).xy-centre;
    return max(radius,max(dot(pa,pa),max(dot(pb,pb),dot(pc,pc))));
}
fn vsm_indices(batch:u32,primitive:u32)->vec3<u32> {
    let base=primitive*3u;
    return vec3<u32>(primary_indices[batch].data[base],primary_indices[batch].data[base+1u],primary_indices[batch].data[base+2u]);
}
fn vsm_position(batch:u32,index:u32)->vec3<f32> {return primary_vertices[batch].data[index*3u].xyz;}
// One ring triangle from object-space positions; vsm_test culls it against the ray cone.
fn vsm_ring_triangle(r:VsmReceiver,v:array<vec3<f32>,3>,ring:VsmRing)->VsmRing {
    return vsm_test(r,vsm_to_world(v[0],r.slot),vsm_to_world(v[1],r.slot),vsm_to_world(v[2],r.slot),ring);
}
// vsm_own[i] by constant indices only (a runtime index can move it out of registers).
fn vsm_own_at(i:u32)->vec3<f32> {return select(select(vsm_own[2],vsm_own[1],i==1u),vsm_own[0],i==0u);}
fn vsm_corner(w:array<vec3<f32>,3>,p:vec3<f32>)->u32 {
    return select(select(select(3u,2u,vsm_same(w[2],p)),1u,vsm_same(w[1],p)),0u,vsm_same(w[0],p));
}
// The receiver triangle, then every triangle sharing one of its vertex positions, reached
// by walking each vertex fan across the exact-position edge adjacency in both
// directions. Positions, not indices, identify vertices, so fans cross duplicated seam
// vertices as the adjacency does and as vsm_in_ring assumes. A triangle across edge
// (c, c+1) is tested only in the fan of c+1.
fn vsm_ring(r:VsmReceiver,walk:bool)->VsmRing {
    var ring=vsm_ring_triangle(r,vsm_own,VsmRing(0xffu&~vsm_active,0.0));
    var visited=1u;
    let adj=vsm_adjacency(r.batch);
    let rec=vsm_records_of(r.batch);
    for(var c=0u;c<3u*u32(walk);c+=1u) {
        if ring.blocked==0xffu {break;}
        let p=vsm_own_at(c);
        let skip=vsm_own_at((c+1u)%3u);
        var closed=false;
        for(var side=0u;side<2u && !closed;side+=1u) {
            var t=r.primitive;
            // Leaving across edge (c+1) shares {c, c+2}; across edge (c+2) shares {c, c+1}.
            var edge=(c+1u+side)%3u;
            var q=vsm_own_at((c+2u-side)%3u);
            var step=0u;
            var n=vsm_nb_at(r.batch,adj,t,edge);
            for(;step<VSM_FAN_STEPS;step+=1u) {
                if n==VSM_MISSING {vsm_dbg.w|=1u<<17u;break;}
                if n==r.primitive {closed=true;break;}
                var w=vsm_tri_in(rec,r.batch,n);
                let around=vsm_nbs(r.batch,adj,n);
                let cp=vsm_corner(w,p);let cq=vsm_corner(w,q);
                if cp==3u || cq==3u || cp==cq {vsm_dbg.w|=1u<<18u;break;}
                if vsm_corner(w,skip)==3u {ring=vsm_ring_triangle(r,w,ring);visited+=1u;}
                edge=cq;q=w[3u-cp-cq];t=n;
                n=vsm_pick(around,edge);
            }
            if step==VSM_FAN_STEPS {vsm_dbg.w|=1u<<16u;}
        }
    }
    ring.radius=sqrt(ring.radius);
    vsm_ring_tests=visited;
    vsm_dbg.w|=min(visited,0xffffu);
    return ring;
}
// ---- Ring horizons (N8b): skip the ring walk where its triangles cannot block ----
// vsm_horizon writes, per triangle and side of its winding plane, the tangent of the
// steepest elevation at which a ray leaving the triangle can reach any triangle of its vertex
// 1-ring (the fans vsm_ring walks), rounded up. The lookup skips the walk when all eight
// directions rise more steeply on the receiver's side. Elevations are invariant under
// similarity transforms only, so other placements always walk.
struct VsmHorizonPass {batch:u32,count:u32,offset:u32,pad:u32};
@group(0) @binding(22) var<uniform> vsm_horizon_pass:VsmHorizonPass;
@group(0) @binding(23) var<storage,read_write> vsm_horizon_out:array<u32>;
@group(0) @binding(24) var<storage,read> vsm_horizons:array<u32>;
override VSM_HORIZON:bool=true;
// The largest sine of elevation above the plane of normal n among directions from a point
// of triangle vsm_ht to a point of triangle vsm_hq: the norm of n's projection onto the cone
// spanned by g = q_i - t_j (the directions from T to Q). If n is in that cone (Q overhangs
// T: their projections on the plane overlap with Q above) it is 1. Otherwise the projection
// lies on a ray or a two-generator face of the cone, so the largest projection of n onto a
// single generator or a generator pair whose coefficients are non-negative is exact.
var<private> vsm_ht:array<vec3<f32>,3>;
var<private> vsm_hq:array<vec3<f32>,3>;
var<private> vsm_hg:array<vec3<f32>,9>;
fn vsm_hg_store(k:u32,g:vec3<f32>) {
    // Constant-index stores only (docs: runtime-indexed stores misbehaved on NVIDIA).
    switch k {
        case 0u {vsm_hg[0]=g;} case 1u {vsm_hg[1]=g;} case 2u {vsm_hg[2]=g;}
        case 3u {vsm_hg[3]=g;} case 4u {vsm_hg[4]=g;} case 5u {vsm_hg[5]=g;}
        case 6u {vsm_hg[6]=g;} case 7u {vsm_hg[7]=g;} default {vsm_hg[8]=g;}
    }
}
// Whether the plane projections of T and Q overlap in their interiors (separating axes on
// the six edge normals; touching along a shared edge or vertex is not overlap).
fn vsm_overlap(n:vec3<f32>)->bool {
    for(var k=0u;k<6u;k+=1u) {
        var a:vec3<f32>;var b:vec3<f32>;
        if k<3u {a=vsm_ht[k];b=vsm_ht[(k+1u)%3u];} else {a=vsm_hq[k-3u];b=vsm_hq[(k-2u)%3u];}
        let axis=cross(b-a,n);
        var t0=3.4e38;var t1=-3.4e38;var q0=3.4e38;var q1=-3.4e38;
        for(var i=0u;i<3u;i+=1u) {
            let x=dot(vsm_ht[i],axis);let y=dot(vsm_hq[i],axis);
            t0=min(t0,x);t1=max(t1,x);q0=min(q0,y);q1=max(q1,y);
        }
        let tolerance=1e-6*(abs(t0)+abs(t1)+abs(q0)+abs(q1));
        if t1<=q0+tolerance || q1<=t0+tolerance {return false;}
    }
    return true;
}
fn vsm_cone_sine(n:vec3<f32>)->f32 {
    var count=0u;
    var above=false;
    for(var i=0u;i<3u;i+=1u) {
        above=above || dot(vsm_hq[i]-vsm_ht[0],n)>0.0;
        for(var j=0u;j<3u;j+=1u) {
            let g=vsm_hq[i]-vsm_ht[j];
            if dot(g,g)>0.0 {vsm_hg_store(count,g);count+=1u;}
        }
    }
    if !above {return 0.0;}
    if vsm_overlap(n) {return 1.0;}
    var best=0.0;
    for(var a=0u;a<count;a+=1u) {
        let ga=vsm_hg[a];
        let aa=dot(ga,ga);let an=dot(ga,n);
        if an>0.0 {best=max(best,an*inverseSqrt(aa));}
        for(var b=a+1u;b<count;b+=1u) {
            let gb=vsm_hg[b];
            let ab=dot(ga,gb);let bb=dot(gb,gb);let bn=dot(gb,n);
            let det=aa*bb-ab*ab;
            if det<=1e-12*aa*bb {continue;}
            // n's projection onto span(ga, gb): coefficients from the 2x2 Gram system.
            let la=(an*bb-bn*ab)/det;let lb=(bn*aa-an*ab)/det;
            if la>=0.0 && lb>=0.0 {best=max(best,sqrt(max(la*an+lb*bn,0.0)));}
        }
    }
    return min(best,1.0);
}
// The tangent of vsm_cone_sine: what the lookup compares with its directions.
fn vsm_kappa(n:vec3<f32>)->f32 {
    let sine=vsm_cone_sine(n);
    if sine>=0.9999 {return 3.4e38;}
    return sine*inverseSqrt(1.0-sine*sine);
}
@compute @workgroup_size(64) fn vsm_horizon(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=vsm_horizon_pass.count {return;}
    let batch=vsm_horizon_pass.batch;
    let primitive=id.x;
    let own=vsm_triangle(batch,primitive);
    vsm_ht=own;
    let cross_n=cross(own[1]-own[0],own[2]-own[0]);
    var up=3.4e38;
    var down=3.4e38;
    if dot(cross_n,cross_n)>0.0 {
        let n=normalize(cross_n);
        up=0.0;down=0.0;
        // vsm_ring's fans, without its early exit.
        for(var c=0u;c<3u;c+=1u) {
            let p=own[c];
            let skip=own[(c+1u)%3u];
            var closed=false;
            for(var side=0u;side<2u && !closed;side+=1u) {
                var t=primitive;
                var edge=(c+1u+side)%3u;
                var q=own[(c+2u-side)%3u];
                for(var step=0u;step<VSM_FAN_STEPS;step+=1u) {
                    let next=vsm_neighbour(batch,t,edge);
                    if next==VSM_MISSING {break;}
                    if next==primitive {closed=true;break;}
                    let w=vsm_triangle(batch,next);
                    let cp=vsm_corner(w,p);let cq=vsm_corner(w,q);
                    if cp==3u || cq==3u || cp==cq {break;}
                    if vsm_corner(w,skip)==3u {
                        vsm_hq=w;
                        up=max(up,vsm_kappa(n));
                        down=max(down,vsm_kappa(-n));
                    }
                    edge=cq;q=w[3u-cp-cq];t=next;
                }
            }
        }
    }
    // f16 keeps three digits; the margin keeps the bound conservative after rounding.
    let packed=min(vec2<f32>(up,down)*1.01+vec2<f32>(1e-3),vec2<f32>(65504.0));
    vsm_horizon_out[vsm_horizon_pass.offset+primitive]=pack2x16float(packed);
}
// Whether all eight directions rise above the receiver triangle's ring horizon on the ray
// origin's side, so the ring walk cannot block any of them. A mirroring placement puts the
// object's winding side on the world winding's far side, so its sides swap (N8d).
fn vsm_beyond_horizon(r:VsmReceiver,l:vec3<f32>)->bool {
    if !VSM_HORIZON {return false;}
    let m=models[r.slot];
    let x=m[0].xyz;let y=m[1].xyz;let z=m[2].xyz;
    let s=dot(x,x);
    if abs(dot(y,y)-s)>1e-4*s || abs(dot(z,z)-s)>1e-4*s || abs(dot(x,y))>1e-4*s || abs(dot(y,z))>1e-4*s || abs(dot(z,x))>1e-4*s {return false;}
    let h=unpack2x16float(vsm_horizons[vsm.horizon[r.batch/4u][r.batch%4u]+r.primitive]);
    let a=vsm_to_world(vsm_own[0],r.slot);
    let winding=cross(vsm_to_world(vsm_own[1],r.slot)-a,vsm_to_world(vsm_own[2],r.slot)-a);
    if dot(winding,winding)==0.0 {return false;}
    let toward=select(lighting.eye-r.world,l,r.thin);
    let up=dot(winding,toward)>=0.0;
    let n=select(-normalize(winding),normalize(winding),up);
    let bound=select(h.y,h.x,up!=(dot(cross(x,y),z)<0.0));
    for(var j=0u;j<8u;j+=1u) {
        let e=dot(vsm_dir(j),n);
        if (vsm_active&(1u<<j))!=0u && (e<=0.0 || e*e<=bound*bound*(1.0-e*e)) {return false;}
    }
    return true;
}
// Whether a map texel's top triangle shares a vertex position with the receiver's.
fn vsm_in_ring(r:VsmReceiver,id:u32)->bool {
    let placement=(id>>vsm.misc.x)-1u;
    if placement!=r.slot {return false;}
    let primitive=id&((1u<<vsm.misc.x)-1u);
    if primitive==r.primitive {return true;}
    var w=vsm_tri(r.batch,primitive);
    for(var i=0u;i<3u;i+=1u) {
        if vsm_same(w[i],vsm_own[0]) || vsm_same(w[i],vsm_own[1]) || vsm_same(w[i],vsm_own[2]) {return true;}
    }
    return false;
}
// Point-location walk steps per aim from the texel's triangle toward the ray (N8b), at
// the receiver's level; coarser levels' texels span more triangles.
override VSM_WALK_STEPS:u32=8u;
// The coarser levels' walk lengthening: VSM_WALK_STEPS << min(level - receiver's, this).
override VSM_WALK_SHIFT:u32=2u;
// N8e lean walks (0: off) and their step bound.
override VSM_LEAN:u32=0u;
override VSM_LEAN_STEPS:u32=2u;
// Crossing refinements: walk aims after the sample's own xy.
override VSM_REFINE:u32=3u;
// The continuity rule's reach: the predicted crossing must lie this many spans of the last
// located triangle beyond the sample (0: any ray still below after the refinements).
override VSM_SPAN:f32=1.5;
// N8c: share the near steps across the eight directions while the cone's footprint stays
// within this many texels, and test the max-mip over the footprint before later steps.
override VSM_SHARE:f32=0.5;
override VSM_MIP:bool=true;
// Directions (bit j) among `mask` that the world-space triangle (a, b, c) blocks: the
// watertight test from each direction's ray origin, from the receiver's epsilon on.
fn vsm_hits_mask(r:VsmReceiver,a:vec3<f32>,b:vec3<f32>,c:vec3<f32>,mask:u32)->u32 {
    var hits=0u;
    if r.thin {
        for(var j=0u;j<8u;j+=1u) {
            if (mask&(1u<<j))==0u {continue;}
            let o=vsm_ray_origin(r,j);
            if vsm_hits_sheared(vsm_shear_at(j),vsm_permute(a-o),vsm_permute(b-o),vsm_permute(c-o),r.epsilon) {hits|=1u<<j;}
        }
    } else {
        let o=vsm_shared_origin;
        let pa=vsm_permute(a-o);let pb=vsm_permute(b-o);let pc=vsm_permute(c-o);
        for(var j=0u;j<8u;j+=1u) {
            if (mask&(1u<<j))!=0u && vsm_hits_sheared(vsm_shear_at(j),pa,pb,pc,r.epsilon) {hits|=1u<<j;}
        }
    }
    return hits;
}
fn vsm_triangle_mask(r:VsmReceiver,rec:vec2<u32>,batch:u32,primitive:u32,m:mat4x4<f32>,mask:u32)->u32 {
    let v=vsm_tri_in(rec,batch,primitive);
    return vsm_hits_mask(r,(m*vec4<f32>(v[0],1.0)).xyz,(m*vec4<f32>(v[1],1.0)).xyz,(m*vec4<f32>(v[2],1.0)).xyz,mask);
}
// The exact decision of the surface the texel's triangle `id` belongs to (N8b), for the rays
// of `mask` around one aim. Scan triangles are often smaller than a texel, so the triangle
// rasterized at the texel centre need not be the one above the ray. Walk across the
// exact-position adjacency to the triangle under the aim's light-space xy (leaving each
// triangle across the edge the point is most outside of), testing every triangle visited,
// and the located triangle's edge neighbours, exactly against every direction in `mask`
// (N8c; N8b tested one direction). If the aim height `q.z` is below the located triangle's
// plane, the ray `step3` rises to it farther along its drift: predict that crossing xy, walk
// there, and repeat, so the crossing triangle itself is tested. x: the blocked directions;
// it stops early once direction `primary` (8: every direction of `mask`) is blocked. Bit 8:
// after the refinements `step3`'s ray is still exactly below an interior triangle and the
// predicted crossing lies beyond the triangles tested (VSM_SPAN spans of the last), so the
// surface is taken to continue there (N8b's continuity rule). Never for the receiver's own
// triangle or 1-ring (tested over the whole ray), at an open edge (the ray may escape), or
// for a ray grazing a crest. Bit 9: the walk ended with the ray exactly below the last
// located interior triangle, which y names (the shared walk's cue for per-direction ones).
fn vsm_top_hits(r:VsmReceiver,id:u32,placement:u32,mask:u32,primary:u32,q:vec3<f32>,step3:vec3<f32>,steps:u32)->vec2<u32> {
    let batch=primary_placements[placement]&0x1fffffffu;
    let adj=vsm_adjacency(batch);
    let rec=vsm_records_of(batch);
    let m=models[placement];
    let stop=select(mask,1u<<primary,primary<8u);
    var t=id&((1u<<vsm.misc.x)-1u);
    var previous=VSM_MISSING;
    // The current walk aim and the ray's light-space height there.
    var aim=q.xy;
    var ray=q.z;
    var below=false;
    var span=0.0;
    var hits=0u;
    // N8e lean walks: past VSM_LEAN times the walk cap in visits, a receiver's walks test
    // the sample's located surface without refinements (its later candidates stay exact
    // where they are tested, but are no longer chased to their crossings).
    let lean=VSM_LEAN!=0u && vsm.n8e.z!=0u && vsm_visits>=VSM_LEAN*vsm.n8e.z;
    let reach=select(steps,min(steps,VSM_LEAN_STEPS),lean);
    for(var phase=0u;phase<=select(VSM_REFINE,0u,lean);phase+=1u) {
        var located=false;
        var height=0.0;
        var own=false;
        // Whether the walk stopped at an open edge, a fold or a degenerate projection rather
        // than running out of steps (a crossing farther than the walk reaches).
        var stopped=false;
        for(var step=0u;step<reach;step+=1u) {
            if VSM_DEBUG {vsm_dbg.y+=select(1u,0x10000u,primary<8u);}
            vsm_visits+=1u;
            let v=vsm_tri_in(rec,batch,t);
            let around=vsm_nbs(batch,adj,t);
            let a=(m*vec4<f32>(v[0],1.0)).xyz;let b=(m*vec4<f32>(v[1],1.0)).xyz;let c=(m*vec4<f32>(v[2],1.0)).xyz;
            hits|=vsm_hits_mask(r,a,b,c,mask&~hits);
            if (hits&stop)==stop {return vec2<u32>(hits,t);}
            let la=vsm_light(a);let lb=vsm_light(b);let lc=vsm_light(c);
            let area=(lb.x-la.x)*(lc.y-la.y)-(lb.y-la.y)*(lc.x-la.x);
            if area==0.0 {stopped=true;break;}
            // Signed sub-areas opposite each vertex; all non-negative (after orientation) inside.
            let w=vec3<f32>((lc.x-lb.x)*(aim.y-lb.y)-(lc.y-lb.y)*(aim.x-lb.x),
                (la.x-lc.x)*(aim.y-lc.y)-(la.y-lc.y)*(aim.x-lc.x),
                (lb.x-la.x)*(aim.y-la.y)-(lb.y-la.y)*(aim.x-la.x))*sign(area);
            if min(w.x,min(w.y,w.z))>=0.0 {
                located=true;
                height=dot(w,vec3<f32>(la.z,lb.z,lc.z))/(w.x+w.y+w.z);
                span=max(distance(la.xy,lb.xy),max(distance(lb.xy,lc.xy),distance(lc.xy,la.xy)));
                own=placement==r.slot && (vsm_same(v[0],vsm_own[0]) || vsm_same(v[0],vsm_own[1]) || vsm_same(v[0],vsm_own[2])
                    || vsm_same(v[1],vsm_own[0]) || vsm_same(v[1],vsm_own[1]) || vsm_same(v[1],vsm_own[2])
                    || vsm_same(v[2],vsm_own[0]) || vsm_same(v[2],vsm_own[1]) || vsm_same(v[2],vsm_own[2]));
                break;
            }
            // Leave across the edge the aim is most outside of; at a fold, where that
            // edge leads back, the other outside edge.
            var edge=0u;
            if w.y<w.x && w.y<=w.z {edge=1u;} else if w.z<w.x && w.z<w.y {edge=2u;}
            var n=vsm_pick(around,edge);
            if n==previous {
                let e1=(edge+1u)%3u;let e2=(edge+2u)%3u;
                let other=select(e2,e1,w[e1]<w[e2]);
                n=select(VSM_MISSING,vsm_pick(around,other),w[other]<0.0);
                if n==previous {stopped=true;break;}
            }
            if n==VSM_MISSING {stopped=true;break;}
            previous=t;t=n;
        }
        var interior=true;
        for(var e=0u;e<3u;e+=1u) {
            let n=vsm_nb_at(batch,adj,t,e);
            interior=interior && n!=VSM_MISSING;
            if n!=VSM_MISSING && n!=previous {
                hits|=vsm_triangle_mask(r,rec,batch,n,m,mask&~hits);
                if (hits&stop)==stop {return vec2<u32>(hits,t);}
            }
        }
        // P-NSQ: the sample's own walk ran out of steps before locating the triangle under
        // its aim, so this candidate is undecided for the walk's directions (bit 10).
        if !located && !stopped && phase==0u {hits|=0x400u;}
        // A refinement aim beyond the walk's reach keeps the previous phase's verdict.
        if !located && !stopped && phase>0u {break;}
        below=false;
        if !located || own || !interior {break;}
        let gap=height-ray;
        if gap<=0.0 {break;}
        below=true;
        // The ray reaches this plane's height after rising `gap`; its xy drifts meanwhile.
        aim+=step3.xy*(gap/step3.z);
        ray+=gap;
    }
    if VSM_CONTINUE && below && distance(aim,q.xy)>VSM_SPAN*span {hits|=0x100u;}
    if below {hits|=0x200u;}
    return vec2<u32>(hits,t);
}
// ---- Max-mips of the top layer (N8c): per clip level, mips 1..log2(side) concatenated ----
@group(0) @binding(35) var<storage,read> vsm_mips:array<f32>;
@group(0) @binding(36) var<storage,read_write> vsm_mips_out:array<f32>;
// First word of `mip` (>= 1) of clip `level`: the mips before it hold (side^2 - s^2)/3
// words, s the side of mip - 1.
fn vsm_mip_base(level:u32,mip:u32)->u32 {
    let side=vsm.counts.y*VSM_PAGE;
    let s=side>>(mip-1u);
    return level*((side*side-1u)/3u)+(side*side-s*s)/3u;
}
// One mip (vsm_pass.level) of every clip level (z): the largest normalized top height of
// the 2x2 cells below. Dispatched on every page-work frame; a mip cell inside one page is
// rewritten only where that page was rasterized this frame (N8f), since the others' top
// words are unchanged.
@compute @workgroup_size(8,8) fn vsm_mip(@builtin(global_invocation_id) id:vec3<u32>) {
    let mip=vsm_pass.level;let level=id.z;
    if level>=vsm.counts.x {return;}
    let side=vsm.counts.y*VSM_PAGE;let s=side>>mip;
    if any(id.xy>=vec2<u32>(s)) {return;}
    if (1u<<mip)<=VSM_PAGE {
        let pages=vsm.counts.y;
        let page=(id.xy<<vec2<u32>(mip))/VSM_PAGE;
        if vsm_render[(level*pages+page.y)*pages+page.x]==0u {return;}
    }
    var h=0.0;
    for(var k=0u;k<4u;k+=1u) {
        let c=id.xy*2u+vec2<u32>(k&1u,k>>1u);
        if mip==1u {
            h=max(h,bitcast<f32>(vsm_top_words[(level*side+c.y)*side+c.x].y));
        } else {
            h=max(h,vsm_mips_out[vsm_mip_base(level,mip-1u)+c.y*(2u*s)+c.x]);
        }
    }
    vsm_mips_out[vsm_mip_base(level,mip)+id.y*s+id.x]=h;
}
// Whether no top-layer texel centre a ray can sample within `radius` of `centre` (light xy)
// at clip `level` is above `lower`: the max over at most 2x2 cells of the mip whose cells
// span the footprint. Conservative: false at window edges.
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

// Per direction, the last map triangle decided (N8b's repeat filter).
var<private> vsm_last:array<u32,8>;
fn vsm_last_store(j:u32,id:u32) {
    // Constant-index stores only (docs: runtime-indexed stores misbehaved on NVIDIA).
    switch j {
        case 0u {vsm_last[0]=id;} case 1u {vsm_last[1]=id;} case 2u {vsm_last[2]=id;} case 3u {vsm_last[3]=id;}
        case 4u {vsm_last[4]=id;} case 5u {vsm_last[5]=id;} case 6u {vsm_last[6]=id;} default {vsm_last[7]=id;}
    }
}
// vsm_last[j] by constant indices only (8: any).
fn vsm_last_at(j:u32)->u32 {
    return select(select(select(vsm_last[7],vsm_last[6],j==6u),select(vsm_last[5],vsm_last[4],j==4u),j<6u),
        select(select(vsm_last[3],vsm_last[2],j==2u),select(vsm_last[1],vsm_last[0],j==0u),j<2u),j<4u);
}
// Light-space step of direction j per unit ray parameter.
fn vsm_step(j:u32)->vec3<f32> {
    let d=vsm_dir(j);
    return vec3<f32>(dot(d,vsm.u.xyz),dot(d,vsm.v.xyz),dot(d,vsm.l.xyz));
}
// One map texel's candidates for the rays of `open` sampled at light-space `q` (N8b): the
// top layer, then the peel layer below it while that is above the ray too, each decided
// exactly by vsm_top_hits unless it is the receiver's own triangle, a 1-ring member already
// tested over the whole ray, or `last`. `j` < 8 decides one direction's sample (stopping at
// its first block, continuity applying to it); 8 decides a shared sample of every open
// direction around the central ray. `foreign` decides only other placements' surfaces.
// Returns the newly blocked directions and the last candidate tested.
fn vsm_texel_hits(r:VsmReceiver,ring:f32,p:vec2<f32>,cell:vec2<i32>,level:u32,lower:f32,open:u32,j:u32,q:vec3<f32>,step3:vec3<f32>,steps:u32,last:u32,foreign:bool)->vec2<u32> {
    let bits=vsm.misc.x;
    let texel=vsm_texel(level);
    let bias=f32(vsm.misc.z)*vsm.depth.z*5.960464477539063e-8;
    let word=vsm_word(cell,level);
    let top_word=vsm_top_words[word];
    let top=fma(bitcast<f32>(top_word.y),vsm.depth.z,vsm.depth.x);
    if top<=lower+bias {return vec2<u32>(0u,last);}
    var blocked=0u;
    var tested=last;
    var above=0u;
    for(var layer=0u;layer<select(1u,2u,VSM_PEEL);layer+=1u) {
        var id=0u;
        if layer==0u {
            id=top_word.x;
            above=id>>bits;
        } else {
            let peel_word=vsm_peel_words[word];
            let peel=fma(bitcast<f32>(peel_word.y),vsm.depth.z,vsm.depth.x);
            if peel<=lower+bias {break;}
            id=peel_word.x;
            // A surface hidden under its own placement is usually a fold of it, whose
            // crossings the top walk decides; `peel_own` tests those too (scan overhangs).
            if (vsm.misc.w&2u)==0u && id>>bits==above {break;}
        }
        if id==0u {continue;}
        let placement=(id>>bits)-1u;
        // Ring members were tested exactly over the whole ray; without the ring walk only
        // the receiver's own triangle was, and its neighbours are decided here.
        if placement==r.slot && (id&((1u<<bits)-1u))==r.primitive {continue;}
        if foreign && placement==r.slot {continue;}
        if (vsm.misc.w&1u)!=0u && placement==r.slot && distance(q.xy,p)<=ring+2.0*texel && vsm_in_ring(r,id) {continue;}
        if id==tested {continue;}
        tested=id;
        if VSM_DEBUG {vsm_dbg.z+=select(1u,0x10000u,j<8u);}
        // One direction's sample tests its own ray (N8b); a shared one every open ray.
        let mask=select(open&~blocked,(1u<<j)&open,j<8u);
        // Attempt 0 is the sample's walk. After a shared walk that ended with the central ray
        // exactly below an interior surface, which each open ray crosses (or escapes) along
        // its own drift, attempts 1-8 refine each from the located triangle with its own
        // continuity. One call site keeps the walk inlined once.
        var located=id;
        var refine=false;
        for(var attempt=0u;attempt<9u;attempt+=1u) {
            var m=mask;var k=j;var qk=q;var sk=step3;
            if attempt>0u {
                k=attempt-1u;
                if !refine || ((open&~blocked)&(1u<<k))==0u {continue;}
                sk=vsm_step(k);
                let pk=vsm_light(vsm_ray_origin(r,k));
                qk=vec3<f32>(pk.xy+sk.xy*((q.z-pk.z)/sk.z),q.z);
                m=1u<<k;
            }
            let found=vsm_top_hits(r,located,placement,m,k,qk,sk,steps);
            let hits=found.x;
            if (hits&0x400u)!=0u {vsm_undecided|=m;}
            if attempt>0u {
                if (hits&0x1ffu)!=0u {blocked|=1u<<k;}
                continue;
            }
            blocked|=hits&0xffu;
            if j<8u {
                if (hits&0x100u)!=0u {blocked|=1u<<j;}
                break;
            }
            refine=(hits&0x200u)!=0u;
            if !refine {break;}
            located=(id&~((1u<<bits)-1u))|found.y;
        }
        if (open&~blocked)==0u || (j<8u && (blocked&(1u<<j))!=0u) {break;}
    }
    return vec2<u32>(blocked,tested);
}
// The far march (N8b's schedule, N8c's order): heights above the receiver's origin at
// geometric steps from the origin through the receiver's level, then two per coarser level,
// up to the highest caster. Step i covers the ray heights since the previous step, the
// first from the origin itself (contacts of other meshes inside `near`); a texel is a
// candidate when a layer surface is above that range (and above the receiver's own plane at
// the texel centre), decided exactly by vsm_top_hits. Steps whose cone footprint (the eight
// directions' samples) stays within VSM_SHARE texels are shared: each texel any direction
// can sample there (the 2x2 block around the ray at the first step) is checked once, and a
// candidate's walk is tested against every open direction. Later steps first test the
// max-mip over the footprint (VSM_MIP); only where a surface may be above the ray does each
// open direction sample its own texel, as N8b did. Returns the directions left open.
fn vsm_march(r:VsmReceiver,ring:f32,level0:u32,open0:u32)->u32 {
    var open=open0;
    vsm_undecided=0u;
    let p=vsm_light(vsm_shared_origin);
    let near=vsm.params.x;
    // The cone's radius per unit height around the central ray, and the thin origins'
    // offsets from the central one.
    var drift=0.0;var offset=0.0;
    for(var j=0u;j<8u;j+=1u) {
        let s=vsm_step(j);
        if (vsm_active&(1u<<j))!=0u {
            drift=max(drift,length(s.xy)/max(s.z,1e-6));
            offset=max(offset,distance(vsm_light(vsm_ray_origin(r,j)).xy,p.xy));
        }
        vsm_last_store(j,0u);
    }
    // N8e walk cap: past this many walked triangles, a receiver with a proven block takes
    // every undecided direction as blocked (VSM_CAP_RULE 1: once half are proven; 2: also
    // stop a receiver with none proven, its undecided directions lit).
    let cap=vsm.n8e.z;
    let majority=(countOneBits(vsm_active)+1u)/2u;
    // Heights past every caster; at least one step, since a contact closer than `near`
    // still needs its sample.
    let clear=max(vsm.depth.w-p.z,1.0001*near);
    let size=i32(vsm.counts.y*VSM_PAGE);
    // The origin-side receiver plane in light space, for texel-centre extrapolation.
    let toward=select(lighting.eye-r.world,vsm.l.xyz,r.thin);
    let n=select(-r.geometric,r.geometric,dot(r.geometric,toward)>=0.0);
    let nl=vec3<f32>(dot(n,vsm.u.xyz),dot(n,vsm.v.xyz),dot(n,vsm.l.xyz));
    let facing=nl.z>0.0;
    let gradient=-nl.xy/max(nl.z,0.05);
    let bias=f32(vsm.misc.z)*vsm.depth.z*5.960464477539063e-8;
    let samples=max(vsm.misc.y,1u);
    var shared_last=0u;
    var h=near;
    var below=select(p.z+near,p.z,VSM_ORIGIN_STEP);
    for(var level=level0;level<select(level0+1u,vsm.counts.x,VSM_ASCEND);level+=1u) {
    let end=min(max(vsm_cap(level),1.0001*near),clear);
    if end<=h {continue;}
    let count=select(VSM_EXTRA_SAMPLES,samples,level==level0);
    let ratio=pow(end/h,1.0/f32(count));
    let texel=vsm_texel(level);
    let lo=vsm.levels[level].origin.xy;
    let steps=select(VSM_WALK_STEPS,vsm.nsq.w,vsm.nsq.w!=0u)<<min(level-level0,VSM_WALK_SHIFT);
    for(var k=0u;k<count;k+=1u) {
        h*=ratio;
        let start=below;
        below=p.z+h;
        let radius=offset+drift*h;
        let block=vsm_block_mode()!=0u && level==level0 && k==0u;
        let shared_step=radius<=VSM_SHARE*texel;
        if !shared_step && VSM_MIP && vsm_mip_clear(level,p.xy,radius,start+bias) {continue;}
        // Shared: every cell a direction's sample (or its first-step 2x2 block) can touch,
        // decided once for all open rays. Otherwise each open direction's own cell (or 2x2
        // block), as N8b. One call site keeps vsm_texel_hits inlined once.
        let x=(p.xy-lo)/texel;
        let reach=vec2<f32>(radius/texel);
        let a=vec2<i32>(floor(x-reach-vec2<f32>(select(0.0,0.5,block))));
        let b=vec2<i32>(floor(x+reach-vec2<f32>(select(0.0,0.5,block))))+vec2<i32>(select(0,1,block));
        let width=u32(b.x-a.x+1);
        let cells=select(select(1u,4u,block),width*u32(b.y-a.y+1),shared_step);
        let slots=select(8u*cells,cells,shared_step);
        for(var slot=0u;slot<slots;slot+=1u) {
            var j=8u;
            var cell=a+vec2<i32>(i32(slot%width),i32(slot/width));
            var q=vec3<f32>(p.xy,p.z+h);
            var s=vec3<f32>(0.0,0.0,1.0);
            var o=p;
            if !shared_step {
                j=slot/cells;
                if (open&(1u<<j))==0u {continue;}
                s=vsm_step(j);
                o=vsm_light(vsm_ray_origin(r,j));
                q=vec3<f32>(o.xy+s.xy*(h/s.z),o.z+h);
                let c=slot%cells;
                cell=select(vec2<i32>(floor((q.xy-lo)/texel)),vec2<i32>(floor((q.xy-lo)/texel-vec2<f32>(0.5)))+vec2<i32>(i32(c&1u),i32(c>>1u)),block);
            }
            if any(cell<vec2<i32>(0)) || any(cell>=vec2<i32>(size)) {continue;}
            var lower=start-p.z+o.z;
            if facing {
                let centre=lo+(vec2<f32>(cell)+vec2<f32>(0.5))*texel;
                lower=max(lower,o.z+dot(gradient,centre-o.xy));
            }
            let last=select(vsm_last_at(j),shared_last,j==8u);
            let foreign=block && vsm_block_mode()==2u && any(cell!=vec2<i32>(floor((q.xy-lo)/texel)));
            let result=vsm_texel_hits(r,ring,o.xy,cell,level,lower,open,j,q,s,steps,last,foreign);
            open&=~result.x;
            if j==8u {shared_last=result.y;} else {vsm_last_store(j,result.y);}
            if open==0u {return 0u;}
            if cap!=0u && vsm_visits>=cap {
                let proven=countOneBits(vsm_active&~open);
                if proven>=select(1u,majority,VSM_CAP_RULE==1u) {return 0u;}
                // Rule 2: a receiver with no proven block stops at the cap, lit where undecided.
                if VSM_CAP_RULE==2u && proven==0u {return open;}
            }
        }
    }
    if h>=clear {break;}
    }
    // P-NSQ `truncated_walks_block`: an open direction that a candidate walk could not decide
    // takes that map texel's verdict (its surface is above the ray there): blocked. Every
    // other open direction marched to its end with each candidate decided: open.
    return open&~select(0u,vsm_undecided,vsm.nsq.z!=0u);
}

// ---- The lookup passes ----
// Cost screens only: 1 writes every covered pixel lit with no other work, 2 also sets up
// the receiver and its eight directions.
override VSM_SKIP:u32=0u;
struct VsmPixel {valid:bool,r:VsmReceiver,level:u32,l:vec3<f32>};
// One receiver's setup: its record, Interactive's eight directions, its own triangle,
// origin and ray permutation. invalid: unlit or not shadowed (visibility 256).
fn vsm_prepare(id:vec2<u32>)->VsmPixel {
    var out:VsmPixel;
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
    let slot=record.x-1u;
    out.r=VsmReceiver(world.xyz,normal.xyz,world.w,normal.w<0.0,slot,primary_placements[slot]&0x1fffffffu,record.y);
    let l=-light.travel_outer.xyz;
    let rows=transpose(view.world_to_clip);let u=normalize(rows[0].xyz);let v=normalize(rows[1].xyz);
    let seed=vsm_hash(id.x+id.y*size.x);
    vsm_seq_shift=vec2<f32>(f32(seed>>8u),f32(vsm_hash(seed^0xa511e9b3u)>>8u))*(1.0/16777216.0);
    vsm_seq_l=l;vsm_seq_u=u;vsm_seq_v=v;
    vsm_seq_slope=view.texel.z;
    // N8e: the first k directions of the eight are evaluated.
    vsm_active=0xffu>>(8u-vsm.n8e.x);
    vsm_own=vsm_tri(out.r.batch,out.r.primitive);
    vsm_shared_origin=vsm_origin(out.r,l);
    vsm_directions(0u);
    out.valid=true;
    out.level=textureLoad(vsm_level_in,p,0).x;
    out.l=l;
    return out;
}
// P-NSQ: batch b's eight directions of the prepared pixel's sequence (vsm.nsq), and their rays.
fn vsm_directions(b:u32) {
    let base=vsm.nsq.y+8u*b;
    let shift=vsm_seq_shift;let l=vsm_seq_l;let u=vsm_seq_u;let v=vsm_seq_v;let slope=vsm_seq_slope;
    // Constant-index stores only (docs: runtime-indexed stores misbehaved on NVIDIA).
    vsm_dir_store(0u,vsm_direction(base,shift,l,u,v,slope));vsm_dir_store(1u,vsm_direction(base+1u,shift,l,u,v,slope));
    vsm_dir_store(2u,vsm_direction(base+2u,shift,l,u,v,slope));vsm_dir_store(3u,vsm_direction(base+3u,shift,l,u,v,slope));
    vsm_dir_store(4u,vsm_direction(base+4u,shift,l,u,v,slope));vsm_dir_store(5u,vsm_direction(base+5u,shift,l,u,v,slope));
    vsm_dir_store(6u,vsm_direction(base+6u,shift,l,u,v,slope));vsm_dir_store(7u,vsm_direction(base+7u,shift,l,u,v,slope));
    vsm_prepare_rays(l);
}
// Directions a pixel evaluates this frame: n8e.x per batch, nsq.x batches.
fn vsm_total()->u32 {return vsm.n8e.x*vsm.nsq.x;}
// P-NSQ: the prepared pixel's visible directions over the frame's batches `first`, `first` +
// `stride`, ... (P-NSQ3: the Surface walk splits a pixel's batches over several lanes; every
// other caller takes them all, 0 and 1). Each batch is N8e's lookup of eight directions (its walk
// cap counts that batch's visits) and depends on nothing another batch computed, so any split
// gives each batch the same result. vsm_batch_work is the pixel's ring tests and walk visits over
// these batches (the walk orders' estimate).
var<private> vsm_batch_work:u32;
fn vsm_walk_batches(px:VsmPixel,first:u32,stride:u32)->u32 {
    let r=px.r;
    var visible=0u;
    vsm_batch_work=0u;
    for(var b=first;b<vsm.nsq.x;b+=stride) {
        if b>0u {vsm_directions(b);}
        vsm_visits=0u;
        let ring=vsm_ring(r,(vsm.misc.w&1u)!=0u && !vsm_beyond_horizon(r,px.l));
        var blocked=ring.blocked;
        if VSM_DEBUG && b==0u {vsm_dbg.x=ring.blocked|(min(px.level,0xffu)<<24u);}
        if VSM_FAR && px.level!=VSM_NONE && blocked!=0xffu {
            let open=vsm_march(r,ring.radius,px.level,0xffu&~blocked);
            if VSM_DEBUG && b==0u {vsm_dbg.x|=((0xffu&~blocked)&~open)<<8u;}
            blocked=0xffu&~open;
        }
        visible+=countOneBits(vsm_active&~blocked);
        vsm_batch_work+=vsm_ring_tests+vsm_visits;
    }
    return visible;
}
override VSM_CAP_RULE:u32=0u;
// The pixel's count from `visible` of the frame's directions (vsm_total).
fn vsm_visible(id:vec2<u32>,visible:u32) {
    let k=vsm_total();
    textureStore(vsm_output,vec2<i32>(id),vec4<u32>((256u*visible+k/2u)/k));
}
// N8f: whether this frame's moved casters can change pixel id's count under a kept camera:
// it lies in one's old or new pixel bounds (its receiver may differ), or its receiver's march
// neighbourhood at a level it marches (vsm_mark's page requests) overlaps one's light-space
// bounds. Nothing else the count depends on changed.
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
// N8e rolling refresh: on a frame that keeps the last frame's camera, casters and settings,
// one trace tile of every a x b block (n8e.y: a, b, then the tile's offset in the block) is
// looked up again and the others keep their last counts in the output. Otherwise every tile.
// A rolling frame that moved casters (N8f) visits every tile: the block's tile and the pixels
// the casters reach are looked up, the others keep their counts.
fn vsm_lookup_tile(group:vec2<u32>,local:vec2<u32>) {
    let size=textureDimensions(vsm_world);
    let rolling=vsm.n8e.y;
    let block=vec2<u32>(rolling&15u,(rolling>>4u)&15u);
    let offset=vec2<u32>((rolling>>8u)&15u,(rolling>>12u)&15u);
    let casters=(vsm.n8e.w&1u)!=0u;
    var tile=group;
    if (rolling>>16u)!=0u && !casters {tile=tile*block+offset;}
    let id=tile*VSM_TRACE_TILE+local;
    vsm_dbg=vec4<u32>(0u);
    if any(id>=size) {return;}
    if textureLoad(vsm_tiles,tile,0).x==0u {return;}
    if casters && any(tile%block!=offset) && !vsm_reached(id) {return;}
    if VSM_SKIP==1u {textureStore(vsm_output,vec2<i32>(id),vec4<u32>(256u));return;}
    let px=vsm_prepare(id);
    if !px.valid {textureStore(vsm_output,vec2<i32>(id),vec4<u32>(256u));return;}
    let r=px.r;
    if VSM_SKIP==2u {textureStore(vsm_output,vec2<i32>(id),vec4<u32>(u32(dot(vsm_shear_at(r.primitive&7u),vsm_own[0])>1e30)+256u));return;}
    vsm_visible(id,vsm_walk_batches(px,0u,1u));
    // The tile's work for the next frame's order: its heaviest lane's, where it is not light.
    let work=vsm_batch_work;
    if VSM_PERSIST!=0u && VSM_ORDER && !casters && work>=VSM_HEAVY {
        atomicMax(&vsm_scratch.words[VSM_HEAD+vsm_tile_count()+tile.y*vsm_tile_grid().x+tile.x],work);
    }
    if VSM_DEBUG {
        let index=VSM_HEAD+2u*vsm_tile_count()+8u*(id.x+id.y*size.x);
        atomicStore(&vsm_scratch.words[index],vsm_dbg.x);atomicStore(&vsm_scratch.words[index+1u],vsm_dbg.y);
        atomicStore(&vsm_scratch.words[index+2u],vsm_dbg.z);atomicStore(&vsm_scratch.words[index+3u],vsm_dbg.w);
        for(var k=4u;k<8u;k+=1u) {atomicStore(&vsm_scratch.words[index+k],0u);}
    }
}
// N8m (docs/non-rt-n8m.md): the lookup's schedule. The encoder launches VSM_PERSIST workgroups
// (more than any GPU keeps resident; 0 launches one per trace tile instead), and each
// takes the grid's tiles from a queue until none is left: a finished tile's warp starts the next
// at once, rather than leaving its slot empty until a new workgroup launches. With VSM_ORDER the
// queue hands tiles out heaviest first (vsm_order_*), so the slowest do not start last and run
// alone at the end. Only the schedule changes: every pixel's count is the same whichever warp
// computes it.
override VSM_PERSIST:u32=4096u;
override VSM_ORDER:bool=true;
// Tiles whose heaviest lane did less work than this (ring tests plus walk visits) are light:
// they keep the grid's order, after the heavy ones.
const VSM_HEAVY:u32=16u;
var<workgroup> vsm_wg_tile:u32;
// The lookup's grid this frame: every trace tile, or on a rolling frame that did not move
// casters one tile per refresh block (encode_virtual's dispatch).
fn vsm_lookup_grid()->vec2<u32> {
    let rolling=vsm.n8e.y;
    var block=vec2<u32>(1u);
    if (rolling>>16u)!=0u && (vsm.n8e.w&1u)==0u {block=vec2<u32>(rolling&15u,(rolling>>4u)&15u);}
    return (vsm_tile_grid()+block-vec2<u32>(1u))/block;
}
@compute @workgroup_size(8,4) fn vsm_lookup(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_id) local:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    vsm_lane=lane;
    if VSM_PERSIST==0u {vsm_lookup_tile(group.xy,local.xy);return;}
    let grid=vsm_lookup_grid();
    let tiles=grid.x*grid.y;
    loop {
        // The next tile is claimed only once this warp's last one is done: claimed earlier, it
        // would wait behind a slow tile (measured: a longer tail, and other register choices).
        if lane==0u {vsm_wg_tile=atomicAdd(&vsm_scratch.queue,1u);}
        let t=workgroupUniformLoad(&vsm_wg_tile);
        if t>=tiles {break;}
        // Casters-only frames (N8f) keep the grid's order: their work is where the casters
        // reach, which the order (recorded by the last other frame) does not predict.
        var g=t;
        if VSM_ORDER && (vsm.n8e.w&1u)==0u {g=atomicLoad(&vsm_scratch.words[VSM_HEAD+t]);}
        vsm_lookup_tile(vec2<u32>(g%grid.x,g/grid.x),local.xy);
    }
}
// ---- The tile order (N8m), before a persistent lookup with VSM_ORDER ----
// Grid tile g's bucket: the bit length of the work its trace tile's last lookup recorded (0 for
// light or never-looked-up tiles), at most VSM_BUCKETS - 1. Also that work's word, or VSM_NONE
// for a tile outside the image.
fn vsm_order_bucket(g:u32,grid:vec2<u32>)->vec2<u32> {
    let rolling=vsm.n8e.y;
    var tile=vec2<u32>(g%grid.x,g/grid.x);
    if (rolling>>16u)!=0u && (vsm.n8e.w&1u)==0u {
        tile=tile*vec2<u32>(rolling&15u,(rolling>>4u)&15u)+vec2<u32>((rolling>>8u)&15u,(rolling>>12u)&15u);
    }
    let dense=vsm_tile_grid();
    if any(tile>=dense) {return vec2<u32>(0u,VSM_NONE);}
    let word=VSM_HEAD+vsm_tile_count()+tile.y*dense.x+tile.x;
    let work=atomicLoad(&vsm_scratch.words[word]);
    return vec2<u32>(min(VSM_BUCKETS-1u,32u-countLeadingZeros(work)),word);
}
// Per-workgroup bucket counts, so each workgroup adds to the shared counters and cursors once per
// bucket rather than once per tile (same-address atomics serialize).
var<workgroup> vsm_wg_counts:array<atomic<u32>,VSM_BUCKETS>;
var<workgroup> vsm_wg_base:array<u32,VSM_BUCKETS>;
@compute @workgroup_size(256) fn vsm_order_count(@builtin(global_invocation_id) id:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    let grid=vsm_lookup_grid();
    if id.x<grid.x*grid.y {atomicAdd(&vsm_wg_counts[vsm_order_bucket(id.x,grid).x],1u);}
    workgroupBarrier();
    if li<VSM_BUCKETS {
        let n=atomicLoad(&vsm_wg_counts[li]);
        if n!=0u {atomicAdd(&vsm_scratch.words[li],n);}
    }
}
// Places each grid tile after every heavier bucket's tiles (within its own bucket, workgroups in
// arrival order and tiles in arrival order inside them, close to the grid's), and clears its work
// word for this frame's lookup to record.
@compute @workgroup_size(256) fn vsm_order_place(@builtin(global_invocation_id) id:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    let grid=vsm_lookup_grid();
    let inside=id.x<grid.x*grid.y;
    var b=vec2<u32>(0u,VSM_NONE);
    var slot=0u;
    if inside {
        b=vsm_order_bucket(id.x,grid);
        slot=atomicAdd(&vsm_wg_counts[b.x],1u);
    }
    workgroupBarrier();
    if li<VSM_BUCKETS {
        let n=atomicLoad(&vsm_wg_counts[li]);
        var start=0u;
        for(var k=li+1u;k<VSM_BUCKETS;k+=1u) {start+=atomicLoad(&vsm_scratch.words[k]);}
        if n!=0u {start+=atomicAdd(&vsm_scratch.words[VSM_BUCKETS+li],n);}
        vsm_wg_base[li]=start;
    }
    workgroupBarrier();
    if inside {
        atomicStore(&vsm_scratch.words[VSM_HEAD+vsm_wg_base[b.x]+slot],id.x);
        if b.y!=VSM_NONE {atomicStore(&vsm_scratch.words[b.y],0u);}
    }
}
