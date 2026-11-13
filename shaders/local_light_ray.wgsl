// LC2: four uniform disk samples, with TR1 rotating the sequence across frames. A disk
// facing the receiver approximates a spherical emitter's apparent area. Incidence and
// candela stay at the light centre; only visibility is integrated over its source.
// A camera receiver the resolve restarts this frame traces the restart count instead
// (rl_local_rays, set by rl_local_budget): four rays are a penumbra's whole estimate there.
// rl_local_visibility is this budget, or with `local_shadow_budget` the receiver's history
// (shaders/local_light_history.wgsl).
fn rl_local_shadow(light:Light, receiver:Receiver, l:vec3<f32>) -> f32 {
    return rl_local_fade(light.kind.w,rl_local_visibility(light,receiver,l));
}
// A light fading in or out of the frame's ray budget (light_cluster.rs `ray_word`): kind.w's
// top byte is its strength in 255ths, 0 meaning full.
fn rl_local_fade(word:u32, visibility:f32) -> f32 {
    let fade=word>>24u;
    if fade==0u { return visibility; }
    return mix(1.0,visibility,f32(fade)/255.0);
}
// The receiver's budget: one ray to a point source, max(rl_local_rays, 4) disk rays
// otherwise, from this frame's place in the sequence.
fn rl_local_rays_visibility(light:Light, receiver:Receiver, l:vec3<f32>) -> f32 {
    let samples=select(1u,max(rl_local_rays,4u),light.source.x>0.0);
    return rl_local_disk_rays(light,receiver,l,(rl_light_frame.info.z&255u)*4u,samples);
}
// The visible fraction of `samples` rays toward the light's disk at lattice indices
// first, first + 1, ... (the centre at radius zero).
fn rl_local_disk_rays(light:Light, receiver:Receiver, l:vec3<f32>, first:u32, samples:u32) -> f32 {
    // Naga 30: declare the query before control flow and reuse it for every ray.
    var lc_query:ray_query;
    let radius=light.source.x;
    let axis=select(vec3<f32>(1.0,0.0,0.0),vec3<f32>(0.0,1.0,0.0),abs(l.x)>0.9);
    let tangent=unit_normal(cross(l,axis));
    let bitangent=cross(l,tangent);
    let seed=rl_local_hash(bitcast<u32>(receiver.pixel.x)^rl_local_hash(bitcast<u32>(receiver.pixel.y))^(light.kind.w&0xffffffu));
    let shift=vec2<f32>(f32(seed&0xffffu),f32(seed>>16u))*(1.0/65536.0);
    var lit=0.0;
    for(var i=0u;i<samples;i+=1u) {
        var source_point=light.position_range.xyz;
        if radius>0.0 {
            // Callers bound the lattice index so long sessions keep f32 fractional precision.
            let k=first+i;
            let u=fract(shift+vec2<f32>(0.754877666,0.569840296)*f32(k));
            let r=radius*sqrt(u.x);
            let phi=6.28318530718*u.y;
            source_point+=r*(tangent*cos(phi)+bitangent*sin(phi));
        }
        let direction=unit_normal(source_point-receiver.world);
        let origin=shadow_origin(receiver,direction);
        let delta=source_point-origin;
        let distance=length(delta);
        let tmax=distance-max(1e-4*distance,1e-3);
        if !(tmax>receiver.epsilon) { lit+=1.0; continue; }
        rayQueryInitialize(&lc_query,LC_SCENE,RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT,255u,receiver.epsilon,tmax,origin,delta/distance));
        while rayQueryProceed(&lc_query) {}
        lit+=select(0.0,1.0,rayQueryGetCommittedIntersection(&lc_query).kind==0u);
    }
    return lit/f32(samples);
}
