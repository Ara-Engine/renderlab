// Only composed on the opt-in wgpu RT path. The raw producer and bounds together
// occupy ~0.37 MiB at 1920x1080; the history validity tag uses its existing G lane.
struct AtFogDepth { eye:vec4<f32>, size:vec4<u32>, halves:array<u32> };
struct AtFogBounds {
    enabled:atomic<u32>, skipped:atomic<u32>, zero_density:atomic<u32>, pad:u32,
    first_hidden:array<u32>,
};
@group(0) @binding(21) var<storage,read> at_fog_cull_depth:AtFogDepth;
@group(0) @binding(22) var<storage,read_write> at_fog_bounds:AtFogBounds;
fn at_fog_cull_enabled()->bool {return atomicLoad(&at_fog_bounds.enabled)!=0u;}
fn at_fog_cell_hidden(id:vec3<u32>)->bool {
    return at_fog_cull_enabled() && id.z>=at_fog_bounds.first_hidden[id.x+id.y*atmosphere.grid.x];
}
@compute @workgroup_size(8,8) fn at_fog_occlusion(@builtin(global_invocation_id) id:vec3<u32>) {
    let grid=atmosphere.grid.xy;
    if any(id.xy>=grid) {return;}
    if all(id.xy==vec2<u32>(0u)) {atomicStore(&at_fog_bounds.enabled,1u);}
    var first=AT_FROXEL_SLICES;
    if all((at_fog_cull_depth.size.zw+vec2<u32>(7u))/8u==grid) {
        var farthest=0u;
        for(var y=-2;y<=2;y+=1) {
            for(var x=-2;x<=2;x+=1) {
                let tile=vec2<u32>(clamp(vec2<i32>(id.xy)+vec2<i32>(x,y),vec2<i32>(0),vec2<i32>(grid)-vec2<i32>(1)));
                for(var part=0u;part<2u;part+=1u) {
                    let row=min(tile.y*2u+part,at_fog_cull_depth.size.y-1u);
                    let bits=at_fog_cull_depth.halves[tile.x+row*at_fog_cull_depth.size.x];
                    farthest=max(farthest,select(bits,0x7f800000u,bits==0u));
                }
            }
        }
        let depth=bitcast<f32>(farthest);
        if depth<atmosphere.volume.x {
            let conservative=depth+max(0.001,depth*1e-5);
            first=min(AT_FROXEL_SLICES,u32(ceil(at_froxel_coordinate(conservative)*f32(AT_FROXEL_SLICES)))+1u);
        }
    }
    at_fog_bounds.first_hidden[id.x+id.y*grid.x]=first;
    if first<AT_FROXEL_SLICES {atomicAdd(&at_fog_bounds.skipped,AT_FROXEL_SLICES-first);}
}
// Check every contributing tap explicitly; interpolating a validity mask could round
// to one with a tiny invalid weight. Never import an unfilled cell through history.
fn at_fog_history_filled(uv:vec3<f32>,grid:vec3<u32>)->bool {
    let p=clamp(uv*vec3<f32>(grid)-vec3<f32>(0.5),vec3<f32>(0.0),vec3<f32>(grid)-vec3<f32>(1.0));
    let base=vec3<i32>(floor(p));let fraction=fract(p);
    for(var z=0;z<2;z+=1) {for(var y=0;y<2;y+=1) {for(var x=0;x<2;x+=1) {
        let offset=vec3<i32>(x,y,z);
        let weight=select(vec3<f32>(1.0)-fraction,fraction,offset!=vec3<i32>(0));
        if all(weight>vec3<f32>(0.0)) {
            let cell=min(base+offset,vec3<i32>(grid)-vec3<i32>(1));
            if textureLoad(at_history,cell,0).g!=1.0 {return false;}
        }
    }}}
    return true;
}
