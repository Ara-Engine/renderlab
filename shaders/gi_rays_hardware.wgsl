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
// ---- see-through placements (docs/design-see-through-occlusion.md) ----
// The TLAS's see-through placements (SEE_THROUGH_MASK) are built on non-opaque copies of their
// meshes: with the switch on (the default; RENDERLAB_GI_SEE_THROUGH=0 turns it off), a closest-hit ray adds their bit to its mask and the traversal hands
// their triangles over as candidates, while opaque geometry still commits in hardware. Each
// candidate's share is kept with its distance (the first GI_SEE_THROUGH_KEPT; past them, their
// product at the farthest), and only those nearer than the committed hit are applied: one
// traversal. A film passes 1 - coverage (the material's factors and its texture's mean alpha,
// for these rays carry diffuse light), glass (IOR above 1) also its Fresnel transmission tinted
// by its base colour, as the live reference's transmitted_within. gi_hit_transmission holds the
// last gi_closest's product.
// The see-through table (src/hardware_shadow_gpu.rs SEE_THROUGH_HEADER_WORDS): the first
// see-through placement, their count, two spare words, then per placement 4 words.
@group(0) @binding(40) var<storage,read> gi_see_through:array<u32>;
const GI_H_SEE_THROUGH:u32=35u;
const GI_SEE_THROUGH_KEPT:u32=4u;
var<private> gi_hit_transmission:vec3<f32>;
// Schlick's dielectric Fresnel reflectance at cosine c for index n.
fn gi_fresnel(n:f32, c:f32) -> f32 {
    let f0=((n-1.0)/(n+1.0))*((n-1.0)/(n+1.0));
    return f0+(1.0-f0)*pow(1.0-clamp(c,0.0,1.0),5.0);
}
fn gi_closest(origin:vec3<f32>, direction:vec3<f32>, tmin:f32, tmax:f32) -> GiHit {
    var query:ray_query<vertex_return>;
    gi_hit_transmission=vec3<f32>(1.0);
    let first=gi_see_through[0];
    let clear=gi_word(GI_H_SEE_THROUGH)!=0u && gi_see_through[1]!=0u;
    var kept_t:array<f32,4>;
    var kept:array<vec3<f32>,4>;
    var count=0u;
    var far_t=-1.0;
    var far=vec3<f32>(1.0);
    if clear {
        rayQueryInitialize(&query,gi_scene,RayDesc(0u,VISIBLE_MASK|SEE_THROUGH_MASK,tmin,tmax,origin,direction));
        while rayQueryProceed(&query) {
            // Only see-through placements are non-opaque: every candidate is one.
            let c=rayQueryGetCandidateIntersection(&query);
            let r=4u+(c.instance_custom_data-first)*4u;
            let rg=unpack2x16float(gi_see_through[r]);
            let bc=unpack2x16float(gi_see_through[r+1u]);
            let ior=bitcast<f32>(gi_see_through[r+2u]);
            var share=vec3<f32>(1.0-bc.y);
            if ior>1.0 {
                let v=getCandidateHitVertexPositions(&query);
                let m=c.world_to_object;
                let ng=normalize(transpose(mat3x3<f32>(m[0],m[1],m[2]))*cross(v[1]-v[0],v[2]-v[0]));
                share*=(1.0-gi_fresnel(ior,abs(dot(ng,direction))))*vec3<f32>(rg,bc.x);
            }
            if count<GI_SEE_THROUGH_KEPT {
                kept_t[count]=c.t;
                kept[count]=share;
                count+=1u;
            } else {
                far*=share;
                far_t=max(far_t,c.t);
            }
        }
    } else {
        rayQueryInitialize(&query,gi_scene,RayDesc(RAY_FLAG_FORCE_OPAQUE,VISIBLE_MASK,tmin,tmax,origin,direction));
        while rayQueryProceed(&query) {}
    }
    let hit=rayQueryGetCommittedIntersection(&query);
    let limit=select(tmax,hit.t,hit.kind!=0u);
    for(var i=0u;i<count;i+=1u) {
        if kept_t[i]<limit { gi_hit_transmission*=kept[i]; }
    }
    if far_t>=0.0 && far_t<limit { gi_hit_transmission*=far; }
    let none=vec2<f32>(0.0);
    if hit.kind==0u { return GiHit(false,0.0,vec3<f32>(0.0),0x7fffffffu,vec3<u32>(0u),none); }
    let v=getCommittedHitVertexPositions(&query);
    let m=hit.world_to_object;
    let n=transpose(mat3x3<f32>(m[0],m[1],m[2]))*cross(v[1]-v[0],v[2]-v[0]);
    return GiHit(true,hit.t,unit_normal(n),hit.instance_custom_data,vec3<u32>(hit.primitive_index,0u,0u),hit.barycentrics);
}
// Sky visibility along an unbounded ray: any hit blocks it. With see-through placements on
// (GI_H_SEE_THROUGH), a pane blocks it too: the direction is then gathered (gi_gather), which
// takes the sky behind the pane at the pane's transmission rather than all of it.
fn gi_visible(origin:vec3<f32>, direction:vec3<f32>) -> bool {
    if gi_word(GI_H_SEE_THROUGH)!=0u && gi_see_through[1]!=0u {
        var query:ray_query<vertex_return>;
        rayQueryInitialize(&query,gi_scene,RayDesc(RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_TERMINATE_ON_FIRST_HIT,SHADOW_CASTER_MASK|SEE_THROUGH_MASK,0.0,1e20,origin,direction));
        while rayQueryProceed(&query) {}
        return rayQueryGetCommittedIntersection(&query).kind==0u;
    }
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
fn gi_hit_primitive(inst:u32, tri:vec3<u32>) -> u32 { return tri.x; }
fn gi_hit_placement(inst:u32) -> u32 { return inst; }
// A gather hit's receiver: the traced shading receiver of its triangle (point, plane normal
// turned toward the ray's origin) and its bin source, the interpolated vertex normal on that
// side, as gi_probe forms a pixel's.
fn gi_hit_receiver(origin:vec3<f32>, d:vec3<f32>, h:GiHit) -> GiReceiver {
    let r=gi_receiver(vec4<u32>(h.inst+1u,h.tri.x,bitcast<vec2<u32>>(h.bary)));
    let viewed=select(-r.geometric,r.geometric,dot(r.geometric,-d)>=0.0);
    return GiReceiver(r.world,viewed,gi_bin_source(viewed,r.vertex));
}
