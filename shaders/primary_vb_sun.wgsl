// U-FIXED: the RT path's sun from the visibility buffer's receivers. Appended to the
// reconstruction's held trace module (temporal_reconstruction.rs `vb_sun_source`), so its sun
// (`primary_sun`: TR1's budget, direction rotation and cold stop, then the sun visibility
// history or the held stop) is the traced Interactive entries'. The receiver comes from the
// buffer's resolve (world and epsilon, geometric normal and coverage) and its placement from
// the hit record; the output keeps the 0..256 contract, misses and skips 256.
// sunrt-5: the 8x4 workgroup is a trace tile, so the tile vote (primary_sun_tile.wgsl) runs here
// too; `vb_sun_source` replaces the marker line with its call in uniform control flow.
@compute @workgroup_size(8,4) fn primary_vb_sun(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(fresh_world);
    let inside=all(id.xy<size);
    let p=vec2<i32>(id.xy);
    var hit=false;
    var receiver:Receiver;
    var slot=0u;
    if inside && textureLoad(primary_tiles,id.xy/PRIMARY_TILE,0).x!=0u {
        let record=textureLoad(primary_hit,p,0);
        let normal=textureLoad(fresh_normal,p,0);
        if record.x!=0u && normal.w!=0.0 {
            let world=textureLoad(fresh_world,p,0);
            slot=record.x-1u;
            receiver=Receiver(world.xyz,normal.xyz,p,slot,world.w,normal.w<0.0);
            hit=true;
        }
    }
    // sunrt tile vote
    if !inside {return;}
    var visible=FRESH_SKIP;
    if hit {visible=primary_sun(receiver,slot,id.xy,size.x);}
    primary_sun_store(p,visible);
    textureStore(fresh_output,p,vec4<u32>(select((256u/tr1_rays)*visible,256u,visible==FRESH_SKIP)));
}
