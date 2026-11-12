// U-FIXED: the RT path's sun from the visibility buffer's receivers. Appended to the
// reconstruction's held trace module (temporal_reconstruction.rs `vb_sun_source`), so its sun
// (`primary_sun`: TR1's budget, direction rotation and cold stop, then the sun visibility
// history or the held stop) is the traced Interactive entries'. The receiver comes from the
// buffer's resolve (world and epsilon, geometric normal and coverage) and its placement from
// the hit record; the output keeps the 0..256 contract, misses and skips 256.
@compute @workgroup_size(8,4) fn primary_vb_sun(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(fresh_world);
    if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);
    var visible=FRESH_SKIP;
    if textureLoad(primary_tiles,id.xy/PRIMARY_TILE,0).x!=0u {
        let record=textureLoad(primary_hit,p,0);
        let normal=textureLoad(fresh_normal,p,0);
        if record.x!=0u && normal.w!=0.0 {
            let world=textureLoad(fresh_world,p,0);
            let slot=record.x-1u;
            let receiver=Receiver(world.xyz,normal.xyz,p,slot,world.w,normal.w<0.0);
            visible=primary_sun(receiver,slot,id.xy,size.x);
        }
    }
    primary_sun_store(p,visible);
    textureStore(fresh_output,p,vec4<u32>(select((256u/tr1_rays)*visible,256u,visible==FRESH_SKIP)));
}
