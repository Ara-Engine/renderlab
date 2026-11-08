// ---- world radiance cache ray source: hardware ray queries (RT path) ----
// The renderer's shadow/primary TLAS, all geometry opaque and both faces blocking,
// as hardware shadow rays treat it. `normal` is the hit triangle's source-winding
// normal in world space (the inverse transpose of the instance transform). The RT path
// has no cooked near field (docs/m7-g1c.md): the hit's triangle and barycentrics are not
// reported, its instance is a placeholder with bit 31 clear (gi_cache.wgsl keeps a side flag
// there), and visibility rays always trace the whole ray.
@group(0) @binding(34) var gi_scene:acceleration_structure<vertex_return>;
struct GiHit { hit:bool, t:f32, normal:vec3<f32>, inst:u32, tri:vec3<u32>, bary:vec2<f32> };
fn gi_closest(origin:vec3<f32>, direction:vec3<f32>, tmin:f32, tmax:f32) -> GiHit {
    var query:ray_query<vertex_return>;
    rayQueryInitialize(&query,gi_scene,RayDesc(RAY_FLAG_FORCE_OPAQUE,255u,tmin,tmax,origin,direction));
    while rayQueryProceed(&query) {}
    let hit=rayQueryGetCommittedIntersection(&query);
    let none=vec2<f32>(0.0);
    if hit.kind==0u { return GiHit(false,0.0,vec3<f32>(0.0),0x7fffffffu,vec3<u32>(0u),none); }
    let v=getCommittedHitVertexPositions(&query);
    let m=hit.world_to_object;
    let n=transpose(mat3x3<f32>(m[0],m[1],m[2]))*cross(v[1]-v[0],v[2]-v[0]);
    return GiHit(true,hit.t,unit_normal(n),0x7fffffffu,vec3<u32>(0u),none);
}
// Sky visibility along an unbounded ray: any hit blocks it.
fn gi_visible(origin:vec3<f32>, direction:vec3<f32>) -> bool {
    var query:ray_query<vertex_return>;
    rayQueryInitialize(&query,gi_scene,RayDesc(RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_TERMINATE_ON_FIRST_HIT,255u,0.0,1e20,origin,direction));
    while rayQueryProceed(&query) {}
    return rayQueryGetCommittedIntersection(&query).kind==0u;
}
fn gi_near_base(inst:u32) -> u32 { return 0u; }
fn gi_near_blocked(base:u32, tri:vec3<u32>, bary:vec2<f32>, back:bool, d:vec3<f32>, bins:u32) -> bool { return false; }
