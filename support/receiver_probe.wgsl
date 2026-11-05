// Test-only extra bindings: retain the production ray query and estimator body.
@group(2) @binding(0) var<storage,read_write> probe_masks:array<atomic<u32>>;
@group(2) @binding(1) var<storage,read_write> probe_rays:array<vec4<f32>>;
@group(2) @binding(2) var probe_scene:acceleration_structure<vertex_return>;
var<private> probe_seed:u32;
var<private> probe_witness:bool;
@compute @workgroup_size(8) fn receiver_probe(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    probe_seed=group.x;
    probe_witness=true;
    let pixel=vec2<u32>(1295u,513u);let p=vec2<i32>(pixel);
    let world=textureLoad(fresh_world,p,0);let normal=textureLoad(fresh_normal,p,0);
    let receiver=Receiver(world.xyz,normal.xyz,p,0u,world.w,normal.w<0.0);
    let count=fresh_estimate(receiver,pixel,1920u,lane);
}

@group(2) @binding(3) var<storage,read> probe_pixels:array<u32>;
// Per receiver: sum(blocked), sum(blocked^2), partial replicas, replicas.
@group(2) @binding(4) var<storage,read_write> probe_stats:array<atomic<u32>>;
var<workgroup> probe_blocked:atomic<u32>;
@compute @workgroup_size(8) fn receiver_verify(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    probe_seed=group.x+1u;
    let index=probe_pixels[group.y];let pixel=vec2<u32>(index%1920u,index/1920u);let p=vec2<i32>(pixel);
    let world=textureLoad(fresh_world,p,0);let normal=textureLoad(fresh_normal,p,0);
    let receiver=Receiver(world.xyz,normal.xyz,p,0u,world.w,normal.w<0.0);
    let visible=fresh_estimate(receiver,pixel,1920u,lane);
    atomicAdd(&probe_blocked,select(32u-visible,0u,visible==FRESH_SKIP));
    workgroupBarrier();
    if lane==0u {
        let blocked=atomicLoad(&probe_blocked);let base=group.y*4u;
        atomicAdd(&probe_stats[base],blocked);
        atomicAdd(&probe_stats[base+1u],blocked*blocked);
        atomicAdd(&probe_stats[base+2u],select(0u,1u,blocked>0u && blocked<256u));
        atomicAdd(&probe_stats[base+3u],1u);
    }
}
