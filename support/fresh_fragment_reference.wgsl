// Test-only cold fragment reference, extracted from the frozen M6 reference.
// No history reads, packing, reconstruction, or temporal sample-index advance.
@group(0) @binding(2) var hardware_scene:acceleration_structure;
var<private> reference_pixel:vec2<u32>;
fn hash_u32(input:u32) -> u32 {
    var x=input; x=(x^(x>>16u))*0x7feb352du; x=(x^(x>>15u))*0x846ca68bu; return x^(x>>16u);
}
fn disk_direction(index:u32, shift:vec2<f32>, l:vec3<f32>, u:vec3<f32>, v:vec3<f32>, slope:f32) -> vec3<f32> {
    let r=fract(vec2<f32>(f32(index))*vec2<f32>(0.7548776662466927,0.5698402909980532)+shift);
    let a=6.283185307179586*r.y;
    return normalize(l+slope*sqrt(r.x)*(cos(a)*u+sin(a)*v));
}
fn nearest_shadow_hit(view:ShadowView, origin:vec3<f32>, projected:vec3<f32>, direction:vec3<f32>) -> f32 {
    let delta=(view.world_to_clip*vec4<f32>(direction,0.0)).xyz;
    let rate=-delta.z;
    if rate<=0.0 { return 0.0; }
    let tmin=view.coverage_bias.z/rate;
    var tmax=projected.z/rate;
    for(var a=0u;a<2u;a+=1u) {
        if delta[a]>0.0 { tmax=min(tmax,(1.0-projected[a])/delta[a]); }
        if delta[a]<0.0 { tmax=min(tmax,(-1.0-projected[a])/delta[a]); }
    }
    if tmax<=tmin { return 0.0; }
    var query:ray_query;
    rayQueryInitialize(&query,hardware_scene,RayDesc(0u,255u,tmin,tmax,origin,direction));
    while rayQueryProceed(&query) {}
    let hit=rayQueryGetCommittedIntersection(&query);
    return select(hit.t,0.0,hit.kind==0u);
}
fn hard_shadow(light:Light, world:vec3<f32>, geometric:vec3<f32>, l:vec3<f32>) -> f32 {
    if light.kind.z==0u { return 1.0; }
    var layer=light.kind.y; var found=false;
    let depth=dot(shadow_views[layer].camera_depth,vec4<f32>(world,1.0));
    for(var i=0u;i<light.kind.z;i+=1u) {
        let coverage=shadow_views[light.kind.y+i].coverage_bias;
        if depth>=coverage.x && depth<=coverage.y { layer=light.kind.y+i; found=true; break; }
    }
    if !found { return 1.0; }
    let view=shadow_views[layer];
    let normal=select(-geometric,geometric,dot(geometric,l)>=0.0);
    let origin=world+normal*view.coverage_bias.w;
    let projected=(view.world_to_clip*vec4<f32>(origin,1.0)).xyz;
    if any(abs(projected.xy)>vec2<f32>(1.0)) || projected.z<0.0 || projected.z>1.0 { return 1.0; }
    let rows=transpose(view.world_to_clip); let u=normalize(rows[0].xyz); let v=normalize(rows[1].xyz);
    let seed=hash_u32(reference_pixel.x+reference_pixel.y*REFERENCE_WIDTH);
    let shift=vec2<f32>(f32(seed>>8u),f32(hash_u32(seed^0xa511e9b3u)>>8u))*(1.0/16777216.0);
    var visible=0.0;
    for(var j=0u;j<256u;j+=1u) {
        let direction=disk_direction(j,shift,l,u,v,view.texel.z);
        let distance=nearest_shadow_hit(view,origin,projected,direction);
        visible+=select(0.0,1.0,distance==0.0);
    }
    return visible/256.0;
}
