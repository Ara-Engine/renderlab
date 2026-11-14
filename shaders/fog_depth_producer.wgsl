// Current opaque radial depth per 8x4 primary trace group. Zero means unwritten;
// infinity means a miss, thin receiver, or a subsequently repaired/retried pixel.
struct RlFogDepth { eye:vec4<f32>, size:vec4<u32>, halves:array<atomic<u32>> };
@group(0) @binding(81) var<storage,read_write> rl_fog_depth:RlFogDepth;
var<workgroup> rl_fog_farthest:atomic<u32>;
fn rl_fog_depth_group(id:vec2<u32>,group:vec2<u32>,record:vec4<u32>,point:vec3<f32>) {
    var bits=0x7f800000u;
    if all(id<rl_fog_depth.size.zw) && record.x!=0u {
        let distance=length(point-rl_fog_depth.eye.xyz);
        if (primary_placements[record.x-1u]&0x20000000u)==0u && distance>0.0 && distance<3.4e38 {
            bits=bitcast<u32>(distance);
        }
    }
    atomicMax(&rl_fog_farthest,bits);
    workgroupBarrier();
    if all(id==group*vec2<u32>(8u,4u)) {
        atomicStore(&rl_fog_depth.halves[group.x+group.y*rl_fog_depth.size.x],atomicLoad(&rl_fog_farthest));
    }
}
fn rl_fog_depth_changed(pixel:vec2<u32>) {
    let half_tile=pixel/vec2<u32>(8u,4u);
    atomicMax(&rl_fog_depth.halves[half_tile.x+half_tile.y*rl_fog_depth.size.x],0x7f800000u);
}
