// sunrt-6: the ray-traced fog injection with a sun vote, composed by atmosphere_gpu.rs in place of
// at_fog_inject when the fog cull is on (RENDERLAB_FOG_VOTE=0 keeps the original). Every froxel of a
// 4x4x4 group traces its first sample's sun ray as before; when every first sample with density in
// the group agrees (all lit or all shadowed), the group's later samples take that answer instead of
// a ray. Any disagreement traces every sample, exactly as the original. Hidden cells, the zero-density
// count and the history-validity lane are the fog cull's, unchanged.
var<workgroup> at_fog_votes:atomic<u32>;
@compute @workgroup_size(4,4,4) fn at_fog_inject(@builtin(global_invocation_id) id:vec3<u32>) {
    let grid=vec3<u32>(atmosphere.grid.xy,AT_FROXEL_SLICES);
    let inside=all(id<grid);
    let hidden=inside && at_fog_cell_hidden(id);
    let traced=inside && !hidden;
    // A decorrelated pair in opposite half cells covers narrow gaps without a
    // coherent slice/column pattern. These are volume samples, not sun-disk rays.
    let seed=id.x*73856093u^id.y*19349663u^id.z*83492791u;
    let shift=vec3<f32>(f32(seed&1023u),f32((seed>>10u)&1023u),f32((seed>>20u)&1023u))/1024.0;
    let j=fract(at_jitter(atmosphere.grid.z)+shift);
    var lit_density=0.0;var total_density=0.0;
    let base_samples=u32(atmosphere.volume.w);
    // A reset (including a moving sun) cannot reuse history: spend on fresh samples.
    let samples=select(base_samples,2u*base_samples,atmosphere.previous_eye.w==0.0);
    // Vote: 1 a first sample shadowed, 2 a first sample lit.
    var vote=0u;
    if traced {
        let at=at_fog_sample(id,grid,j,0u,samples);
        let world=at.xyz;
        let extinction=at_fog_sample_extinction(at);
        if extinction>0.0 {
            let visible=at_sun_visible(world);
            lit_density+=extinction*visible;
            vote=select(1u,2u,visible>0.5);
        }
        total_density+=extinction;
    }
    if vote!=0u {atomicOr(&at_fog_votes,vote);}
    workgroupBarrier();
    let votes=atomicLoad(&at_fog_votes);
    if !traced {
        if hidden {textureStore(at_scatter_out,vec3<i32>(id),vec4<f32>(0.0));}
        return;
    }
    let agreed=votes==1u || votes==2u;
    let agreed_visible=select(0.0,1.0,votes==2u);
    for(var sample=1u;sample<samples;sample+=1u) {
        let at=at_fog_sample(id,grid,j,sample,samples);
        let world=at.xyz;
        let extinction=at_fog_sample_extinction(at);
        if extinction>0.0 {
            var visible=agreed_visible;
            if !agreed {visible=at_sun_visible(world);}
            lit_density+=extinction*visible;
        }
        total_density+=extinction;
    }
    if total_density==0.0 && at_fog_cull_enabled() {atomicAdd(&at_fog_bounds.zero_density,1u);}
    textureStore(at_scatter_out,vec3<i32>(id),vec4<f32>(lit_density/max(total_density,1e-20),select(0.0,1.0,at_fog_cull_enabled()),0.0,total_density/f32(samples)));
}
// A froxel sample's world position and its distance along the view ray (at_fog_inject's
// arithmetic, unchanged).
fn at_fog_sample(id:vec3<u32>,grid:vec3<u32>,j:vec3<f32>,sample:u32,samples:u32)->vec4<f32> {
    let offset=fract(j+vec3<f32>(f32(sample)/f32(samples)));
    let uv=(vec2<f32>(id.xy)+offset.xy)/vec2<f32>(grid.xy);
    let d=at_view_direction(at_uv_ndc(uv));
    let near=at_froxel_distance(f32(id.z)/f32(grid.z));
    let far=at_froxel_distance(f32(id.z+1u)/f32(grid.z));
    let distance=mix(near,far,offset.z);
    return vec4<f32>(atmosphere.eye.xyz+d*distance,distance);
}
fn at_fog_sample_extinction(at:vec4<f32>)->f32 {
    return select(0.0,at_fog_density(at.y)+atmosphere.volume.y,at.w>=atmosphere.fog.w);
}
