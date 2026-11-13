// Exact cold R2 sequence. Each pixel starts at index zero on every frame.
@group(0) @binding(2) var fresh_scene:acceleration_structure;
@group(0) @binding(3) var fresh_world:texture_2d<f32>;
@group(0) @binding(4) var fresh_normal:texture_2d<f32>;
@group(0) @binding(5) var fresh_output:texture_storage_2d<r32uint,write>;
@group(0) @binding(6) var fresh_visibility:texture_2d<u32>;
// The receiver pixel's fresh count, for every shadowed light (lights[0]'s estimate).
fn hard_shadow(light:Light, receiver:Receiver, l:vec3<f32>) -> f32 {
    if light.kind.z==0u { return 1.0; }
    return f32(textureLoad(fresh_visibility,receiver.pixel,0).x)/256.0;
}
struct FreshReceiver {
    @location(0) world:vec4<f32>,
    @location(1) normal:vec4<f32>,
};
@fragment fn fs_receiver(input:RasterInput, triangle:RasterTriangle, @builtin(front_facing) front:bool) -> FreshReceiver {
    if material.double_sided==0u && front!=(input.winding>0.0) {discard;}
    let receiver=raster_receiver(input.world,triangle,vec2<i32>(input.position.xy),material.thin!=0u);
    // world.w is epsilon; normal.w is coverage with the material's thin sign.
    return FreshReceiver(vec4<f32>(receiver.world,receiver.epsilon),vec4<f32>(receiver.geometric,select(1.0,-1.0,receiver.thin)));
}
fn fresh_hash(input:u32) -> u32 {
    var x=input;x=(x^(x>>16u))*0x7feb352du;x=(x^(x>>15u))*0x846ca68bu;return x^(x>>16u);
}
fn fresh_direction(index:u32,shift:vec2<f32>,l:vec3<f32>,u:vec3<f32>,v:vec3<f32>,slope:f32)->vec3<f32> {
    let r=fract(vec2<f32>(f32(index))*vec2<f32>(0.7548776662466927,0.5698402909980532)+shift);
    let a=6.283185307179586*r.y;
    return normalize(l+slope*sqrt(r.x)*(cos(a)*u+sin(a)*v));
}
// Exact uses eight lanes per receiver. The 8-direction prefix uses one lane.
override FRESH_DIRECTIONS:u32=256u;
const FRESH_LANES:u32=8u;
// Receiver outside every cascade or light-space box: shaded as fully visible.
const FRESH_SKIP:u32=0xffffffffu;
// Visible rays among this lane's share, or FRESH_SKIP. Every lane takes the same exits.
fn fresh_estimate(receiver:Receiver, pixel:vec2<u32>, width:u32, lane:u32) -> u32 {
    let world=receiver.world;
    let light=lights[0];
    let l=-light.travel_outer.xyz;
    var layer=light.kind.y;
    let depth=dot(shadow_views[layer].camera_depth,vec4<f32>(world,1.0));
    var found=false;
    for(var i=0u;i<light.kind.z;i+=1u) {
        let coverage=shadow_views[light.kind.y+i].coverage_bias;
        if depth>=coverage.x && depth<=coverage.y {layer=light.kind.y+i;found=true;break;}
    }
    if !found {return FRESH_SKIP;}
    let view=shadow_views[layer];
    let origin=shadow_origin(receiver,l);
    let projected=(view.world_to_clip*vec4<f32>(origin,1.0)).xyz;
    if any(abs(projected.xy)>vec2<f32>(1.0)) || projected.z<0.0 || projected.z>1.0 {return FRESH_SKIP;}
    let rows=transpose(view.world_to_clip);let u=normalize(rows[0].xyz);let v=normalize(rows[1].xyz);
    let seed=fresh_hash(pixel.x+pixel.y*width);
    let shift=vec2<f32>(f32(seed>>8u),f32(fresh_hash(seed^0xa511e9b3u)>>8u))*(1.0/16777216.0);
    var visible=0u;
    // Function scope avoids Naga 30's loop-local opaque null initializer.
    var query:ray_query;
    for(var j=lane;j<FRESH_DIRECTIONS;j+=select(FRESH_LANES,1u,FRESH_DIRECTIONS==8u)) {
        let direction=fresh_direction(j,shift,l,u,v,view.texel.z);
        var ray_origin=origin;var ray_projected=projected;
        if receiver.thin {
            ray_origin=shadow_origin(receiver,direction);
            ray_projected=(view.world_to_clip*vec4<f32>(ray_origin,1.0)).xyz;
        }
        let delta=(view.world_to_clip*vec4<f32>(direction,0.0)).xyz;
        let rate=-delta.z;
        var blocked=false;
        if rate>0.0 {
            let tmin=receiver.epsilon;
            var tmax=ray_projected.z/rate;
            for(var axis=0u;axis<2u;axis+=1u) {
                if delta[axis]>0.0 {tmax=min(tmax,(1.0-ray_projected[axis])/delta[axis]);}
                if delta[axis]<0.0 {tmax=min(tmax,(-1.0-ray_projected[axis])/delta[axis]);}
            }
            if tmax>tmin {
                // Epsilon is strictly positive: any committed hit blocks this ray.
                // Neither a closest distance nor a dynamic first-hit flag is needed.
                let flags=RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_TERMINATE_ON_FIRST_HIT;
                rayQueryInitialize(&query,fresh_scene,RayDesc(flags,1u,tmin,tmax,ray_origin,direction));
                while rayQueryProceed(&query) {}
                blocked=rayQueryGetCommittedIntersection(&query).kind!=0u;
            }
        }
        visible+=select(1u,0u,blocked);
    }
    return visible;
}
// Receiver pixels per workgroup; fresh_shadow_gpu.rs TILE must match.
const FRESH_TILE=vec2<u32>(2u,2u);
var<workgroup> fresh_counts:array<atomic<u32>,32>;
// Exact: 2x2 receivers, eight lanes each. Interactive: 8x4, one lane each.
// Both retain the same per-pixel direction sequence and 0..256 output contract.
@compute @workgroup_size(32) fn fresh_compute(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) index:u32) {
    let size=textureDimensions(fresh_world);
    let lanes=select(FRESH_LANES,1u,FRESH_DIRECTIONS==8u);
    let tile=select(FRESH_TILE,vec2<u32>(8u,4u),FRESH_DIRECTIONS==8u);
    let slot=index/lanes;
    let id=group.xy*tile+vec2<u32>(slot%tile.x,slot/tile.x);
    let inside=all(id<size);
    let p=vec2<i32>(id);
    var partial=FRESH_SKIP;
    if inside {
        let normal=textureLoad(fresh_normal,p,0);
        if normal.w!=0.0 {
            let world=textureLoad(fresh_world,p,0);
            partial=fresh_estimate(Receiver(world.xyz,normal.xyz,p,0u,world.w,normal.w<0.0),id,size.x,index%lanes);
        }
    }
    atomicAdd(&fresh_counts[slot],select(partial,0u,partial==FRESH_SKIP));
    workgroupBarrier();
    if inside && index%lanes==0u {
        // Write uncovered pixels too: no earlier frame can supply visibility.
        textureStore(fresh_output,p,vec4<u32>(select((256u/FRESH_DIRECTIONS)*atomicLoad(&fresh_counts[slot]),256u,partial==FRESH_SKIP)));
    }
}
