
// A native, held pixel sees a periodic 256-point jitter lattice. Accumulate a
// bounded count of its two surfaces, then keep that coverage while the geometry
// is unchanged. Surface colours continue normally. Motion/cuts/events use TR1b.
fn tr1_cycle_coverage(p:vec2<i32>,s:Tr1Surface,u:Tr1Surface,occluder_is_s:bool,
    history:vec3<f32>,coverage:f32,weight:f32)->vec3<f32> {
    if !tr1_native() || tr1.flags.x!=0u || tr1.flags.z==0u
        || tr1_near_skin || s.moved || tr1_moved(u.id)
        || tr1.windows.x<256.0 || tr1.windows.y<256.0
        || (s.id!=0u && tr1_relit(s.point.xyz))
        || (u.id!=0u && tr1_relit(u.point.xyz)) {
        return vec3<f32>(coverage,weight,0.0);
    }
    let first=textureLoad(tr1_anchor_in,p,0).x&TR1_ID;
    let old=textureLoad(tr1_layer_in,p,0);
    let second=old.x&TR1_ID;
    let same_pair=(first==s.id && second==u.id)||(first==u.id && second==s.id);
    var n=0.0;var hits=0.0;
    if old.x!=TR1_NONE && same_pair && (old.x&TR1_COVERAGE_COUNT)!=0u {
        let counts=unpack2x16float(old.w);
        n=counts.y;
        hits=select(n-counts.x,counts.x,(first==s.id)==occluder_is_s);
    } else if history.y>0.0 {
        // Preserve a young cumulative history, including the spatial start.
        // A full-age, untagged history has not counted a held cycle: only a
        // small prior survives that transition. Motion already caps its age.
        n=floor(select(history.y,4.0,history.y>=256.0));
        hits=history.x*n;
    }
    if n<256.0 {
        hits+=select(0.0,1.0,occluder_is_s);
        n+=1.0;
    }
    return vec3<f32>(clamp(hits/n,0.0,1.0),n,1.0);
}
