// sunrt-2: the tile vote's history hint in the held trace module (temporal_reconstruction.rs
// `trace_source_held` replaces primary_sun_tile.wgsl's default hint with this). It reads
// U-RTVIS's per-pixel sun history (rtvis_held.wgsl) as rtvis_hold does, but only to choose how
// many probes the tile traces; the probes still vote, so a turning sun may use it
// (rtvis_held.sun.y, set on frames that continue history, is the hinted probe count; 0 is off).
// A still, unskinned receiver outside every moving caster's swept shadow whose reprojected 3x3
// history has been one class for HELD_STREAK frames hints that class: 2 lit, 4 shadowed.
fn tr1_tile_hint(receiver:Receiver,slot:u32)->u32 {
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
    for(var k=0;k<9;k+=1) {
        let h=textureLoad(rtvis_history_in,q+vec2<i32>(k%3-1,k/3-1),0).x;
        if (h>>8u)!=cls || (h&0xffu)<rtvis_held.words.z {return 0u;}
    }
    return select(4u,2u,cls==2u);
}
fn tr1_tile_hint_probes()->u32 {return rtvis_held.sun.y;}
// sunrt-7: rtvis_held.sun.z is set on frames the planner found unchanged (camera, placements,
// poses, sun, restart zones): a hinted receiver's history class is still its answer.
fn tr1_tile_reuse()->bool {return rtvis_held.sun.z!=0u;}
