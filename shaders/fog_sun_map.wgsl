// Current-frame directional depth, independent of camera visibility. Snap the
// world-space footprint to texels so camera translation cannot swim the shafts.
// Preserve half-metre near-field sampling; receivers outside 32 m use exact rays.
@group(0) @binding(23) var<storage,read_write> at_fog_sun_depth:array<f32>;
const AT_FOG_MAP_SIZE:u32=128u;
fn at_fog_basis() -> mat3x3<f32> {
    let sun=atmosphere.sun.xyz;
    let axis=select(vec3<f32>(0.0,1.0,0.0),vec3<f32>(1.0,0.0,0.0),abs(sun.y)>0.95);
    let x=normalize(cross(axis,sun));
    return mat3x3<f32>(x,cross(sun,x),sun);
}
fn at_fog_map_centre(basis:mat3x3<f32>,step:f32) -> vec2<f32> {
    return floor(vec2<f32>(dot(atmosphere.eye.xyz,basis[0]),dot(atmosphere.eye.xyz,basis[1]))/step)*step;
}
@compute @workgroup_size(8,8) fn at_fog_sun_map(@builtin(global_invocation_id) id:vec3<u32>) {
    let basis=at_fog_basis();
    let step=2.0*min(atmosphere.volume.x,32.0)/f32(AT_FOG_MAP_SIZE-4u);
    let xy=at_fog_map_centre(basis,step)+(vec2<f32>(id.xy)+vec2<f32>(0.5)-vec2<f32>(f32(AT_FOG_MAP_SIZE)/2.0))*step;
    if any(id.xy>=vec2<u32>(AT_FOG_MAP_SIZE)) { return; }
    let reach=1e5+atmosphere.volume.x;
    let z=dot(atmosphere.eye.xyz,basis[2])+reach;
    let origin=basis*vec3<f32>(xy,z);
    var query:ray_query;
    var limit=reach+atmosphere.volume.x;
    var found=false;
    var finished=false;
    // Any-hit traversal usually finds the front surface immediately. Test the
    // segment before it, excluding two representable distances to avoid retesting
    // the same coplanar layer. The final empty segment gives a conservative depth.
    for(var layer=0u;layer<8u;layer+=1u) {
        rayQueryInitialize(&query,at_scene,RayDesc(RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_TERMINATE_ON_FIRST_HIT,SHADOW_CASTER_MASK,0.0,limit,origin,-basis[2]));
        while rayQueryProceed(&query) {}
        let hit=rayQueryGetCommittedIntersection(&query);
        if hit.kind==0u {finished=true;break;}
        found=true;
        let bits=bitcast<u32>(hit.t);
        if bits<=2u {limit=0.0;finished=true;break;}
        limit=bitcast<f32>(bits-2u);
    }
    var depth=select(-1e20,z-limit,found);
    if !finished {
        // Bounded work even for deeply layered casters; recover exact depth.
        rayQueryInitialize(&query,at_scene,RayDesc(RAY_FLAG_FORCE_OPAQUE,SHADOW_CASTER_MASK,0.0,reach+atmosphere.volume.x,origin,-basis[2]));
        while rayQueryProceed(&query) {}
        let hit=rayQueryGetCommittedIntersection(&query);
        depth=select(-1e20,z-hit.t,hit.kind!=0u);
    }
    at_fog_sun_depth[id.y*AT_FOG_MAP_SIZE+id.x]=depth;
}
fn at_sun_visible(p:vec3<f32>) -> f32 {
    let basis=at_fog_basis();
    let step=2.0*min(atmosphere.volume.x,32.0)/f32(AT_FOG_MAP_SIZE-4u);
    let xy=vec2<f32>(dot(p,basis[0]),dot(p,basis[1]));
    let uv=(xy-at_fog_map_centre(basis,step))/step+vec2<f32>(f32(AT_FOG_MAP_SIZE)/2.0-0.5);
    let cell=vec2<i32>(floor(uv));
    if any(cell<vec2<i32>(0)) || any(cell>=vec2<i32>(i32(AT_FOG_MAP_SIZE)-1)) { return at_sun_ray_visible(p); }
    let z=dot(p,basis[2])+0.02;
    var visible=0u;
    for(var y=0;y<2;y+=1) { for(var x=0;x<2;x+=1) {
        let pixel=vec2<u32>(cell+vec2<i32>(x,y));
        visible+=u32(z>=at_fog_sun_depth[pixel.y*AT_FOG_MAP_SIZE+pixel.x]);
    }}
    // Resolve depth discontinuities with the original exact query. No PCF
    // light bleeding, temporal map reuse or lag after sun/geometry changes.
    if visible!=0u && visible!=4u { return at_sun_ray_visible(p); }
    return f32(visible)/4.0;
}
