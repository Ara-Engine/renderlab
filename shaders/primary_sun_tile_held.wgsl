// sunrt-2: the tile vote's history hint in the held trace module (temporal_reconstruction.rs
// `trace_source_held` replaces primary_sun_tile.wgsl's default hint with this). It reads
// U-RTVIS's per-pixel sun history (rtvis_held.wgsl) as rtvis_hold does, but only to choose how
// many probes the tile traces; the probes still vote, so a turning sun may use it
// (rtvis_held.sun.y, set on frames that continue history, is the hinted probe count; 0 is off).
// A still, unskinned receiver outside every moving caster's swept shadow whose reprojected 3x3
// history has been one class for HELD_STREAK frames hints that class: 2 lit, 4 shadowed.
// The hinted receiver's streak: the fewest frames any texel of its reprojected 3x3 has held the class.
var<private> tr1_hint_streak:u32=0u;
fn tr1_tile_hint(receiver:Receiver,slot:u32)->u32 {
    tr1_hint_streak=0u;
    if rtvis_held.sun.y==0u {return 0u;}
    let batch=primary_placements[slot]&0x1fffffffu;
    if batch_skinned(primary_batch_info(batch)) || rtvis_moved(slot) || rtvis_swept(receiver.world) {return 0u;}
    let c=rtvis_held.previous*vec4<f32>(receiver.world,1.0);
    if c.w<=0.0 {return 0u;}
    let size=vec2<i32>(textureDimensions(rtvis_history_in));
    let q=vec2<i32>(floor(fma(c.xy/c.w,vec2<f32>(0.5,-0.5),vec2<f32>(0.5))*vec2<f32>(size)));
    if any(q<vec2<i32>(1)) || any(q>=size-vec2<i32>(1)) {return 0u;}
    let cls=textureLoad(rtvis_history_in,q,0).x>>8u;
    if cls==0u {return 0u;}
    var streak=255u;
    for(var k=0;k<9;k+=1) {
        let h=textureLoad(rtvis_history_in,q+vec2<i32>(k%3-1,k/3-1),0).x;
        if (h>>8u)!=cls || (h&0xffu)<rtvis_held.words.z {return 0u;}
        streak=min(streak,h&0xffu);
    }
    tr1_hint_streak=streak;
    return select(4u,2u,cls==2u);
}
fn tr1_tile_hint_probes()->u32 {return rtvis_held.sun.y;}
// sunrt-7: rtvis_held.sun.z is set on frames the planner found unchanged (camera, placements,
// poses, sun, restart zones) to the streak a reused class needs: a hinted receiver whose
// reprojected 3x3 has held its class that many jittered frames keeps it without a ray. The streak
// bounds what the jitter has not yet shown: a surface or shadow covering a larger share of the
// pixel would have broken it.
fn tr1_tile_reuse()->bool {return rtvis_held.sun.z!=0u && tr1_hint_streak>=rtvis_held.sun.z;}

// sunrt-8 compaction. In a tile whose probes disagree, a receiver whose reprojected 3x3 history is
// settled (hinted) and whose own TR1_TILE_PROBES probes all agree with it keeps them: it is not in a
// penumbra by its history, its neighbours' and this frame's directions. Every other receiver is
// listed in the edge worklist's spare upper part (past one item per pixel, which the repair list
// never reaches; only the deferred trace sizes the worklist that large), and primary_sun_continue
// traces the rest of its budget 32 to a group, before the edge repair. The tile's warp no longer
// waits on its slowest receiver. tr1_list counts the items and holds their dispatch arguments.
// A receiver past the list's capacity resumes here, as without the list.
// RENDERLAB_SUN_TILE_LIST=0 compiles TR1_TILE_LIST false: every receiver resumes here.
const TR1_TILE_LIST:bool=true;
struct Tr1List {x:atomic<u32>,y:atomic<u32>,z:u32,count:atomic<u32>};
@group(0) @binding(27) var<storage,read_write> tr1_list:Tr1List;
var<workgroup> tr1_list_mask:atomic<u32>;
var<workgroup> tr1_list_base:u32;
// Called by tr1_tile_vote in uniform control flow, after the tile's votes.
fn tr1_tile_list(hint:u32,pixel:vec2<u32>,width:u32) {
    if !TR1_TILE_LIST {return;}
    let size=textureDimensions(primary_hit_output);
    let first=size.x*size.y;
    let items=arrayLength(&primary_repairs.items);
    let capacity=select(0u,items-first,items>first && first<0x1000000u);
    var listed=false;
    if tr1_tile_state==2u && tr1_list_record.x!=0u && capacity!=0u {
        let settled=tr1_tile_pause==TR1_TILE_PROBES
            && ((hint==2u && tr1_tile_count==tr1_tile_pause) || (hint==4u && tr1_tile_count==0u));
        if settled {tr1_tile_state=3u;} else {listed=true;}
    }
    let lane=(pixel.x%8u)+(pixel.y%4u)*8u;
    if listed {atomicOr(&tr1_list_mask,1u<<lane);}
    workgroupBarrier();
    let mask=atomicLoad(&tr1_list_mask);
    if lane==0u && mask!=0u {
        let n=countOneBits(mask);
        let base=atomicAdd(&tr1_list.count,n);
        tr1_list_base=base;
        let block=(base+n-1u)/32u;
        if base%32u==0u || base/32u!=block {
            atomicMax(&tr1_list.x,min(block+1u,256u));
            atomicMax(&tr1_list.y,block/256u+1u);
        }
        tr1_list.z=1u;
    }
    workgroupBarrier();
    if listed {
        let index=tr1_list_base+countOneBits(mask&((1u<<lane)-1u));
        if index<capacity {
            let word=(pixel.x+pixel.y*width)|(tr1_tile_count<<24u)|(tr1_tile_pause<<28u);
            primary_repairs.items[first+index]=PrimaryRepairItem(word,tr1_list_record.x,tr1_list_record.y,tr1_list_record.z,tr1_list_record.w);
            tr1_tile_state=3u;
        }
    }
}
// The listed receivers' remaining directions, from their probes' count, with the trace entries'
// held stop and history store (indirect from tr1_list).
@compute @workgroup_size(32) fn primary_sun_continue(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let size=textureDimensions(primary_hit_output);
    let first=size.x*size.y;
    let index=(group.x+group.y*256u)*32u+lane;
    if index>=atomicLoad(&tr1_list.count) || first+index>=arrayLength(&primary_repairs.items) {return;}
    let item=primary_repairs.items[first+index];
    let flat=item.pixel&0xffffffu;
    let id=vec2<u32>(flat%size.x,flat/size.x);let p=vec2<i32>(id);
    let slot=item.placement-1u;let batch=primary_placements[slot]&0x1fffffffu;
    let v=primary_triangle_vertices(batch,item.primitive);
    let bary=bitcast<vec2<f32>>(vec2<u32>(item.u,item.v));
    let point=world_position(receiver_point(v[0],v[1],v[2],vec3<f32>(1.0-bary.x-bary.y,bary)),slot).xyz;
    let receiver=primary_receiver(point,v,slot,p);
    tr1_tile_state=2u;tr1_tile_count=(item.pixel>>24u)&15u;tr1_tile_pause=item.pixel>>28u;
    let visible=primary_sun(receiver,slot,id,size.x);
    primary_sun_store(p,visible);
    textureStore(fresh_output,p,vec4<u32>(select((256u/tr1_rays)*visible,256u,visible==FRESH_SKIP)));
}
