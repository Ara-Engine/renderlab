// LC2: four uniform disk samples, with TR1 rotating the sequence across frames. A disk
// facing the receiver approximates a spherical emitter's apparent area. Incidence and
// candela stay at the light centre; only visibility is integrated over its source.
// A camera receiver the resolve restarts this frame traces the restart count instead
// (rl_local_rays, set by rl_local_budget): four rays are a penumbra's whole estimate there.
fn rl_local_shadow(light:Light, receiver:Receiver, l:vec3<f32>) -> f32 {
    return rl_local_fade(light.kind.w,rl_local_rays_visibility(light,receiver,l));
}
// A light fading in or out of the frame's ray budget (light_cluster.rs `ray_word`): kind.w's
// top byte is its strength in 255ths, 0 meaning full.
fn rl_local_fade(word:u32, visibility:f32) -> f32 {
    let fade=word>>24u;
    if fade==0u { return visibility; }
    return mix(1.0,visibility,f32(fade)/255.0);
}
fn rl_local_bayer4(p:vec2<u32>) -> u32 {
    let x=p.x&3u;let y=p.y&3u;
    let hi=(((x^y)&1u)<<3u)|((y&1u)<<2u);
    let lo=((((x>>1u)^(y>>1u))&1u)<<1u)|((y>>1u)&1u);
    return hi|lo;
}
fn rl_local_rays_visibility(light:Light, receiver:Receiver, l:vec3<f32>) -> f32 {
    // Naga 30: declare the query before control flow and reuse it for every ray.
    var lc_query:ray_query;
    let radius=light.source.x;
    let samples=select(1u,max(rl_local_rays,4u),radius>0.0);
    // Spot/rect/disc lights (kind == 2) emit from a face perpendicular to travel_outer.xyz, so
    // keeping their source disk in that plane prevents wide wall-mounted lights from poking
    // shadow samples behind their mounting wall at grazing view angles. Point lights use a
    // receiver-facing disk matching a spherical source.
    let disk_n=select(l,light.travel_outer.xyz,light.kind.x==2u);
    let axis=select(vec3<f32>(1.0,0.0,0.0),vec3<f32>(0.0,1.0,0.0),abs(disk_n.x)>0.9);
    let tangent=unit_normal(cross(disk_n,axis));
    let bitangent=cross(disk_n,tangent);
    let px=vec2<u32>(bitcast<u32>(receiver.pixel.x),bitcast<u32>(receiver.pixel.y));
    let lhash=rl_local_hash(light.kind.w&0xffffffu);
    let shift=vec2<f32>(
        f32(rl_local_bayer4(px+vec2<u32>(lhash&3u,(lhash>>2u)&3u)))*(1.0/16.0),
        f32(rl_local_bayer4(px+vec2<u32>(((lhash>>4u)+2u)&3u,((lhash>>6u)+1u)&3u)))*(1.0/16.0),
    );
    let frame_r2=vec2<f32>(0.754877666,0.569840296)*f32(rl_light_frame.info.z&255u);
    var lit=0.0;
    for(var i=0u;i<samples;i+=1u) {
        var source_point=light.position_range.xyz;
        if radius>0.0 {
            // Equal-area concentric strata with golden-angle azimuth and Bayer+R2 phase: keeps
            // 4-ray penumbrae smooth during camera motion and jumps instead of clumping.
            let u=fract(shift+frame_r2+vec2<f32>(0.3819660113,0.6180339887)*f32(i));
            let r=radius*sqrt((f32(i)+u.x)/f32(samples));
            let phi=6.28318530718*fract(f32(i)*0.6180339887498949+u.y);
            source_point+=r*(tangent*cos(phi)+bitangent*sin(phi));
        }
        let direction=unit_normal(source_point-receiver.world);
        let origin=shadow_origin(receiver,direction);
        let delta=source_point-origin;
        let distance=length(delta);
        let tmax=distance-max(1e-4*distance,1e-3);
        if !(tmax>receiver.epsilon) { lit+=1.0; continue; }
        rayQueryInitialize(&lc_query,LC_SCENE,RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT,SHADOW_CASTER_MASK,receiver.epsilon,tmax,origin,delta/distance));
        while rayQueryProceed(&lc_query) {}
        lit+=select(0.0,1.0,rayQueryGetCommittedIntersection(&lc_query).kind==0u);
    }
    return lit/f32(samples);
}
