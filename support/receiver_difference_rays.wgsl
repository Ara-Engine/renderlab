var<private> difference_t:f32;
var<private> difference_front:u32;
fn difference_store(index:u32,w:array<u32,8>) {
    atomicStore(&footprint_sums[index].accepted,w[0]);atomicStore(&footprint_sums[index].visible,w[1]);
    atomicStore(&footprint_sums[index].opposite,w[2]);atomicStore(&footprint_sums[index].corner,w[3]);
    atomicStore(&footprint_sums[index].edge,w[4]);atomicStore(&footprint_sums[index].interior,w[5]);
    atomicStore(&footprint_sums[index].nearest,w[6]);atomicStore(&footprint_sums[index].rejected,w[7]);
}
// Four 32-byte output rows per selected pixel: actual f32 ray, raw/repaired
// records, and the exact uploaded instance matrix (column order).
@compute @workgroup_size(8,4) fn film_footprint(@builtin(global_invocation_id) id:vec3<u32>) {
    let index=footprint_params.y;if index%4u!=0u || any(id.xy!=vec2<u32>(0u)) {return;}
    let t=footprint_targets[index];film_sample=vec2<f32>(0.5);
    let ray=primary_ray(t.pixel,t.size);let delta=ray.far-ray.near;let direction=delta/length(delta);
    let raw=primary_raw_closest(t.pixel,t.size);let fixed=primary_repair(raw,ray.near,direction);
    let o=bitcast<vec3<u32>>(ray.near);let d=bitcast<vec3<u32>>(direction);
    difference_store(index,array<u32,8>(o.x,o.y,o.z,d.x,d.y,d.z,bitcast<u32>(difference_t),difference_front));
    difference_store(index+1u,array<u32,8>(raw.record.x,raw.record.y,raw.record.z,raw.record.w,fixed.record.x,fixed.record.y,fixed.record.z,fixed.record.w));
    let m=models[raw.record.x-1u];let a=bitcast<vec4<u32>>(m[0]);let b=bitcast<vec4<u32>>(m[1]);
    let c=bitcast<vec4<u32>>(m[2]);let e=bitcast<vec4<u32>>(m[3]);
    difference_store(index+2u,array<u32,8>(a.x,a.y,a.z,a.w,b.x,b.y,b.z,b.w));
    difference_store(index+3u,array<u32,8>(c.x,c.y,c.z,c.w,e.x,e.y,e.z,e.w));
}
