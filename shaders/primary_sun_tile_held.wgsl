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
    tr1_hint_streak=0u;rtvis_motion_rays=rtvis_motion(receiver.world);
    if rtvis_held.sun.y==0u {return 0u;}
    let batch=primary_placements[slot]&0x1fffffffu;
    if batch_skinned(primary_batch_info(batch)) || rtvis_moved(slot) || rtvis_swept(receiver.world) || rtvis_crossed(receiver.world) {return 0u;}
    let c=rtvis_held.previous*vec4<f32>(receiver.world,1.0);
    if c.w<=1e-6 {return 0u;}
    let size=vec2<i32>(textureDimensions(rtvis_history_in));
    let q=vec2<i32>(floor(fma(c.xy/c.w,vec2<f32>(0.5,-0.5),vec2<f32>(0.5))*vec2<f32>(size)));
    if any(q<vec2<i32>(1)) || any(q>=size-vec2<i32>(1)) {return 0u;}
    let prev_depth=rtvis_depth_code(c.w);
    let id_h=rtvis_id_hash(slot);
    let h0=textureLoad(rtvis_history_in,q,0).x;
    if !rtvis_tag_match(h0,prev_depth,id_h) {return 0u;}
    let cls=(h0>>6u)&3u;
    if cls==0u {return 0u;}
    var streak=63u;
    for(var k=0;k<9;k+=1) {
        let h=textureLoad(rtvis_history_in,q+vec2<i32>(k%3-1,k/3-1),0).x;
        if !rtvis_tag_match(h,prev_depth,id_h) || ((h>>6u)&3u)!=cls || (h&63u)<rtvis_held.words.z {return 0u;}
        streak=min(streak,h&63u);
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
