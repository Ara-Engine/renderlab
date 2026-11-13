// Reuse visibility only when the old volume covers this cell and its density agrees.
// Invalid history (cut, sun/caster change), uncovered cells and density edges trace fresh.
fn at_quarter_history(id:vec3<u32>, jitter:vec3<f32>, samples:u32)->vec2<f32> {
    // Rotate whole 4x4x4 groups: scattered active lanes would keep every ray warp busy.
    let tile=id/vec3<u32>(4u);
    if atmosphere.previous_eye.w==0.0 || ((tile.x+3u*tile.y+5u*tile.z+atmosphere.grid.z)&3u)==0u {
        return vec2<f32>(-1.0,0.0);
    }
    let grid=vec3<u32>(atmosphere.grid.xy,AT_FROXEL_SLICES);
    let centre_uv=(vec2<f32>(id.xy)+vec2<f32>(0.5))/vec2<f32>(grid.xy);
    let centre=atmosphere.eye.xyz+at_view_direction(at_uv_ndc(centre_uv))*at_froxel_distance((f32(id.z)+0.5)/f32(grid.z));
    let clip=atmosphere.previous_view_projection*vec4<f32>(centre,1.0);
    if clip.w<=0.0 { return vec2<f32>(-1.0,0.0); }
    let ndc=clip.xy/clip.w;
    let previous=vec3<f32>(ndc.x*0.5+0.5,0.5-ndc.y*0.5,at_froxel_coordinate(length(centre-atmosphere.previous_eye.xyz)));
    let border=vec3<f32>(0.5)/vec3<f32>(grid);
    if any(previous<border) || any(previous>vec3<f32>(1.0)-border) { return vec2<f32>(-1.0,0.0); }
    var density=0.0;
    for(var sample=0u;sample<samples;sample+=1u) {
        let offset=fract(jitter+vec3<f32>(f32(sample)/f32(samples)));
        let uv=(vec2<f32>(id.xy)+offset.xy)/vec2<f32>(grid.xy);
        let d=at_view_direction(at_uv_ndc(uv));
        let near=at_froxel_distance(f32(id.z)/f32(grid.z));
        let far=at_froxel_distance(f32(id.z+1u)/f32(grid.z));
        let distance=mix(near,far,offset.z);
        let world=atmosphere.eye.xyz+d*distance;
        density+=select(0.0,at_fog_density(world.y)+atmosphere.volume.y,distance>=atmosphere.fog.w);
    }
    density/=f32(samples);
    let history=textureSampleLevel(at_history,atmosphere_sampler,previous,0.0);
    if abs(history.a-density)/max(density,1e-6)>0.1 { return vec2<f32>(-1.0,0.0); }
    return vec2<f32>(clamp(history.r,0.0,1.0),density);
}
