// Composed only for an explicitly opted-in device. Camera binding 0 is unchanged.
@group(0) @binding(2) var hardware_scene:acceleration_structure;
fn hard_shadow(light:Light, receiver:Receiver, l:vec3<f32>) -> f32 {
    let world=receiver.world;
    let geometric=receiver.geometric;
    if light.kind.z==0u { return 1.0; }
    var layer=light.kind.y;
    let depth=dot(shadow_views[layer].camera_depth,vec4<f32>(world,1.0));
    var found=false;
    for(var i=0u;i<light.kind.z;i+=1u) {
        let coverage=shadow_views[light.kind.y+i].coverage_bias;
        if depth>=coverage.x && depth<=coverage.y { layer=light.kind.y+i; found=true; break; }
    }
    if !found { return 1.0; }
    let view=shadow_views[layer];
    let origin=shadow_origin(receiver,l);
    let projected=(view.world_to_clip*vec4<f32>(origin,1.0)).xyz;
    if any(abs(projected.xy)>vec2<f32>(1.0)) || projected.z<0.0 || projected.z>1.0 { return 1.0; }
    let delta=(view.world_to_clip*vec4<f32>(l,0.0)).xyz;
    // texel.y is the CPU-validated rate from this exact rounded matrix and light.
    let rate=view.texel.y;
    let tmin=receiver.epsilon;
    var tmax=projected.z/rate;
    for(var a=0u;a<2u;a+=1u) {
        if delta[a]>0.0 { tmax=min(tmax,(1.0-projected[a])/delta[a]); }
        if delta[a]<0.0 { tmax=min(tmax,(-1.0-projected[a])/delta[a]); }
    }
    if tmax<=tmin { return 1.0; }
    var query:ray_query;
    rayQueryInitialize(&query,hardware_scene,RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT,1u,tmin,tmax,origin,l));
    while rayQueryProceed(&query) {}
    return select(0.0,1.0,rayQueryGetCommittedIntersection(&query).kind==0u);
}
