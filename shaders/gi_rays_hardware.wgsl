// ---- world radiance cache ray source: hardware ray queries (RT path) ----
// The renderer's shadow/primary TLAS, all geometry opaque and both faces blocking,
// as hardware shadow rays treat it. `normal` is the hit triangle's source-winding
// normal in world space (the inverse transpose of the instance transform). A hit names its
// placement (the instance's custom data, as traced primaries' hit records do), its primitive
// in tri.x and its barycentrics, so a cell's query point keeps the exact triangle its
// material comes from and a gather's hit decodes to the receiver a pixel there would have
// (G2). The RT path has no cooked near field (docs/m7-g1c.md): bit 31 of the instance stays
// clear (gi_cache.wgsl keeps a side flag there), and visibility rays always trace the whole
// ray.
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
    return GiHit(true,hit.t,unit_normal(n),hit.instance_custom_data,vec3<u32>(hit.primitive_index,0u,0u),hit.barycentrics);
}
// Sky visibility along an unbounded ray: any hit blocks it.
fn gi_visible(origin:vec3<f32>, direction:vec3<f32>) -> bool {
    return gi_visible_within(origin,direction,1e20);
}
// Visibility within tmax (a light's distance): any hit blocks it.
fn gi_visible_within(origin:vec3<f32>, direction:vec3<f32>, tmax:f32) -> bool {
    var query:ray_query<vertex_return>;
    rayQueryInitialize(&query,gi_scene,RayDesc(RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_TERMINATE_ON_FIRST_HIT,SHADOW_CASTER_MASK,0.0,tmax,origin,direction));
    while rayQueryProceed(&query) {}
    return rayQueryGetCommittedIntersection(&query).kind==0u;
}
fn gi_near_base(inst:u32) -> u32 { return 0u; }
fn gi_near_blocked(base:u32, tri:vec3<u32>, bary:vec2<f32>, back:bool, d:vec3<f32>, bins:u32) -> bool { return false; }
// A hit's material identity + 1 (G2): its placement's draw stable id and instance (0 when the
// draw table has none).
fn gi_hit_identity(inst:u32, tri:vec3<u32>) -> u32 {
    let batch=primary_placements[inst]&0x1fffffffu;
    let d=gi_draws[batch];
    if d.x==0u || inst<d.y || inst-d.y>=d.z { return 0u; }
    return ((d.x<<16u)|(inst-d.y))+1u;
}
fn gi_hit_primitive(tri:vec3<u32>) -> u32 { return tri.x; }
// A gather hit's receiver: the traced shading receiver of its triangle (point, plane normal
// turned toward the ray's origin) and its bin source, the interpolated vertex normal on that
// side, as gi_probe forms a pixel's.
fn gi_hit_receiver(origin:vec3<f32>, d:vec3<f32>, h:GiHit) -> GiReceiver {
    let r=gi_receiver(vec4<u32>(h.inst+1u,h.tri.x,bitcast<vec2<u32>>(h.bary)));
    let viewed=select(-r.geometric,r.geometric,dot(r.geometric,-d)>=0.0);
    return GiReceiver(r.world,viewed,gi_bin_source(viewed,r.vertex));
}
