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
// Exact uses eight lanes per receiver. The 8-direction prefix uses one lane.
override FRESH_DIRECTIONS:u32=256u;
const FRESH_LANES:u32=8u;
// Receiver outside every cascade or light-space box: shaded as fully visible.
const FRESH_SKIP:u32=0xffffffffu;
// Visible rays among this lane's share, or FRESH_SKIP. Every lane takes the same exits.
fn fresh_estimate(world:vec3<f32>, geometric:vec3<f32>, pixel:vec2<u32>, width:u32, lane:u32) -> u32 {
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
    let normal=select(-geometric,geometric,dot(geometric,l)>=0.0);
    let origin=world+normal*view.coverage_bias.w;
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
                // Any hit decides visibility. A closest hit at t=0 would count as visible,
                // so stop at the first hit only when tmin>0 excludes that case.
                let flags=select(RAY_FLAG_FORCE_OPAQUE,RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_TERMINATE_ON_FIRST_HIT,tmin>0.0);
                rayQueryInitialize(&query,fresh_scene,RayDesc(flags,255u,tmin,tmax,origin,direction));
                while rayQueryProceed(&query) {}
                let hit=rayQueryGetCommittedIntersection(&query);
                distance=select(hit.t,0.0,hit.kind==0u);
            }
        }
        visible+=select(0u,1u,distance==0.0);
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
            partial=fresh_estimate(textureLoad(fresh_world,p,0).xyz,normal.xyz,id,size.x,index%lanes);
        }
    }
    atomicAdd(&fresh_counts[slot],select(partial,0u,partial==FRESH_SKIP));
    workgroupBarrier();
    if inside && index%lanes==0u {
        // Write uncovered pixels too: no earlier frame can supply visibility.
        textureStore(fresh_output,p,vec4<u32>(select((256u/FRESH_DIRECTIONS)*atomicLoad(&fresh_counts[slot]),256u,partial==FRESH_SKIP)));
    }
}
