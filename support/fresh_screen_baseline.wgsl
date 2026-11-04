// Frozen fresh visibility body from b8dfa62: the screen's unchanged comparison baseline.
// Test-only; production lives in shaders/fresh_shadows.wgsl.
// Exact cold R2 sequence. Each pixel starts at index zero on every frame.
@group(0) @binding(2) var fresh_scene:acceleration_structure;
@group(0) @binding(3) var fresh_world:texture_2d<f32>;
@group(0) @binding(4) var fresh_normal:texture_2d<f32>;
@group(0) @binding(5) var fresh_output:texture_storage_2d<r32uint,write>;
@group(0) @binding(6) var fresh_visibility:texture_2d<u32>;
var<private> fresh_pixel:vec2<i32>;
fn hard_shadow(light:Light, world:vec3<f32>, geometric:vec3<f32>, l:vec3<f32>) -> f32 {
    if light.kind.z==0u { return 1.0; }
    return f32(textureLoad(fresh_visibility,fresh_pixel,0).x)/256.0;
}
struct FreshReceiver {
    @location(0) world:vec4<f32>,
    @location(1) normal:vec4<f32>,
};
@fragment fn fs_receiver(input:VertexOutput) -> FreshReceiver {
    let geometric=unit_normal(cross(unit_normal(dpdx(input.world)),unit_normal(dpdy(input.world))));
    // Float32 retains the winning fragment's inputs without reconstruction.
    return FreshReceiver(vec4<f32>(input.world,input.position.z),vec4<f32>(geometric,1.0));
}
fn fresh_hash(input:u32) -> u32 {
    var x=input;x=(x^(x>>16u))*0x7feb352du;x=(x^(x>>15u))*0x846ca68bu;return x^(x>>16u);
}
fn fresh_direction(index:u32,shift:vec2<f32>,l:vec3<f32>,u:vec3<f32>,v:vec3<f32>,slope:f32)->vec3<f32> {
    let r=fract(vec2<f32>(f32(index))*vec2<f32>(0.7548776662466927,0.5698402909980532)+shift);
    let a=6.283185307179586*r.y;
    return normalize(l+slope*sqrt(r.x)*(cos(a)*u+sin(a)*v));
}
fn fresh_estimate(world:vec3<f32>, geometric:vec3<f32>, pixel:vec2<u32>, width:u32) -> u32 {
    let light=lights[0];
    let l=-light.travel_outer.xyz;
    var layer=light.kind.y;
    let depth=dot(shadow_views[layer].camera_depth,vec4<f32>(world,1.0));
    var found=false;
    for(var i=0u;i<light.kind.z;i+=1u) {
        let coverage=shadow_views[light.kind.y+i].coverage_bias;
        if depth>=coverage.x && depth<=coverage.y {layer=light.kind.y+i;found=true;break;}
    }
    if !found {return 256u;}
    let view=shadow_views[layer];
    let normal=select(-geometric,geometric,dot(geometric,l)>=0.0);
    let origin=world+normal*view.coverage_bias.w;
    let projected=(view.world_to_clip*vec4<f32>(origin,1.0)).xyz;
    if any(abs(projected.xy)>vec2<f32>(1.0)) || projected.z<0.0 || projected.z>1.0 {return 256u;}
    let rows=transpose(view.world_to_clip);let u=normalize(rows[0].xyz);let v=normalize(rows[1].xyz);
    let seed=fresh_hash(pixel.x+pixel.y*width);
    let shift=vec2<f32>(f32(seed>>8u),f32(fresh_hash(seed^0xa511e9b3u)>>8u))*(1.0/16777216.0);
    var visible=0u;
    // Function scope avoids Naga 30's loop-local opaque null initializer.
    var query:ray_query;
    for(var j=0u;j<256u;j+=1u) {
        let direction=fresh_direction(j,shift,l,u,v,view.texel.z);
        let delta=(view.world_to_clip*vec4<f32>(direction,0.0)).xyz;
        let rate=-delta.z;
        var distance=0.0;
        if rate>0.0 {
            let tmin=view.coverage_bias.z/rate;
            var tmax=projected.z/rate;
            for(var axis=0u;axis<2u;axis+=1u) {
                if delta[axis]>0.0 {tmax=min(tmax,(1.0-projected[axis])/delta[axis]);}
                if delta[axis]<0.0 {tmax=min(tmax,(-1.0-projected[axis])/delta[axis]);}
            }
            if tmax>tmin {
                rayQueryInitialize(&query,fresh_scene,RayDesc(0u,255u,tmin,tmax,origin,direction));
                while rayQueryProceed(&query) {}
                let hit=rayQueryGetCommittedIntersection(&query);
                distance=select(hit.t,0.0,hit.kind==0u);
            }
        }
        visible+=select(0u,1u,distance==0.0);
    }
    return visible;
}
@compute @workgroup_size(8,8) fn fresh_compute(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(fresh_world);
    if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);
    let normal=textureLoad(fresh_normal,p,0);
    var visible=256u;
    if normal.w!=0.0 {
        visible=fresh_estimate(textureLoad(fresh_world,p,0).xyz,normal.xyz,id.xy,size.x);
    }
    // Write uncovered pixels too: no earlier frame can supply visibility.
    textureStore(fresh_output,p,vec4<u32>(visible));
}
