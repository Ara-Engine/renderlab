// Test-only per-record probe of film_validate (src/film_accumulation_tests.rs gpu_film_t2_probe),
// appended to the motion kernel module with three seams in film_validate_pixel (PROBE_EDITS).
// Up to three probe pixels (FilmMotion flags y, z, w: (x + 1) | y << 16; zero for none; flags.x
// must enable diagnostics) write, after the per-pixel diagnostics, FILM_PROBE_STRIDE u32 each:
//   header (u32 0..255): magic, pixel, 3x3 geometry/background counts, discontinuity, near and
//     far view distances, the source window, radians per pixel, landed entries, kept count, the
//     implied-record frame index, the nine decoded fresh planes (12 u32 each from 16), the eye
//     (124), the kept records (128, 4 u32 each);
//   entries (from 256, 64 u32 each, in gather order): source pixel, slot and flags, the source
//     record, its landing point and world point, the five plane tests (own, up, left, right,
//     down) with both sides of each inequality, the exact-ray result, and two closest-hit camera
//     rays through the landing point: from the near plane, and around the record's own point.
// The decisions are the production code's; this only records them and traces extra rays.
const FILM_PROBE_STRIDE:u32=8192u;
const FILM_PROBE_HEADER:u32=256u;
const FILM_PROBE_ENTRY:u32=64u;
var<private> film_probe_entries:u32=0u;

fn film_probe_put(probe:u32,k:u32,v:u32) {
    let i=2u*film_motion.size.x*film_motion.size.y+probe*FILM_PROBE_STRIDE+k;
    if (i&1u)==0u {film_diagnostics[i>>1u].x=v;} else {film_diagnostics[i>>1u].y=v;}
}
fn film_probe_f(probe:u32,k:u32,v:f32) {
    film_probe_put(probe,k,bitcast<u32>(v));
}
fn film_probe_v3(probe:u32,k:u32,v:vec3<f32>) {
    film_probe_f(probe,k,v.x);
    film_probe_f(probe,k+1u,v.y);
    film_probe_f(probe,k+2u,v.z);
}
// The probe slot of pixel p, or -1.
fn film_probe_slot(p:vec2<i32>)->i32 {
    if film_motion.flags.x==0u {return -1;}
    for(var j=0;j<3;j+=1) {
        let w=select(select(film_motion.flags.w,film_motion.flags.z,j==1),film_motion.flags.y,j==0);
        if w!=0u && i32(w&0xffffu)-1==p.x && i32(w>>16u)==p.y {return j;}
    }
    return -1;
}
// The closest opaque hit of a ray over [tmin, tmax]: kind, t, placement index, primitive,
// barycentrics and front face, as 7 u32 at k.
fn film_probe_trace(probe:u32,k:u32,origin:vec3<f32>,direction:vec3<f32>,tmin:f32,tmax:f32) {
    var query:ray_query;
    rayQueryInitialize(&query,fresh_scene,RayDesc(RAY_FLAG_FORCE_OPAQUE,255u,tmin,tmax,origin,direction));
    while rayQueryProceed(&query) {}
    let h=rayQueryGetCommittedIntersection(&query);
    film_probe_put(probe,k,h.kind);
    film_probe_f(probe,k+1u,h.t);
    film_probe_put(probe,k+2u,h.instance_custom_data);
    film_probe_put(probe,k+3u,h.primitive_index);
    film_probe_f(probe,k+4u,h.barycentrics.x);
    film_probe_f(probe,k+5u,h.barycentrics.y);
    film_probe_put(probe,k+6u,u32(h.front_face));
}
fn film_probe_header(probe:u32,p:vec2<i32>,planes:ptr<function,array<FilmPlane,9>>,geometry:u32,background:u32,near:f32,far:f32,discontinuity:bool) {
    film_probe_entries=0u;
    film_probe_put(probe,0u,0x50524f42u);
    film_probe_put(probe,1u,u32(p.x));
    film_probe_put(probe,2u,u32(p.y));
    film_probe_put(probe,3u,geometry);
    film_probe_put(probe,4u,background);
    film_probe_put(probe,5u,u32(discontinuity));
    film_probe_f(probe,6u,near);
    film_probe_f(probe,7u,far);
    let w=film_source_window(p);
    film_probe_put(probe,8u,bitcast<u32>(w.x));
    film_probe_put(probe,9u,bitcast<u32>(w.y));
    film_probe_put(probe,10u,bitcast<u32>(w.z));
    film_probe_put(probe,11u,bitcast<u32>(w.w));
    film_probe_f(probe,12u,film_motion.eye.w);
    film_probe_put(probe,13u,0u);
    film_probe_put(probe,15u,film_motion.size.w);
    for(var j=0u;j<9u;j+=1u) {
        let b=16u+12u*j;
        let q=(*planes)[j];
        film_probe_put(probe,b,q.placement);
        film_probe_v3(probe,b+1u,q.world);
        film_probe_v3(probe,b+4u,q.normal);
        film_probe_put(probe,b+7u,u32(q.side));
        film_probe_f(probe,b+8u,q.distance);
        let at=p+vec2<i32>(i32(j%3u)-1,i32(j/3u)-1);
        if film_inside(at) {
            let raw=film_planes[u32(at.x)+u32(at.y)*film_motion.size.x];
            film_probe_put(probe,b+9u,raw.x);
            film_probe_put(probe,b+10u,raw.z);
            film_probe_put(probe,b+11u,raw.w);
        }
    }
    film_probe_v3(probe,124u,film_motion.eye.xyz);
}
fn film_probe_entry(probe:u32,s:vec2<i32>,slot:u32,listed:bool,r:vec4<u32>,moved:vec3<f32>,valid:bool,planes:ptr<function,array<FilmPlane,9>>,discontinuity:bool,count:u32) {
    let n=film_probe_entries;
    film_probe_entries+=1u;
    film_probe_put(probe,13u,film_probe_entries);
    if FILM_PROBE_HEADER+(n+1u)*FILM_PROBE_ENTRY>FILM_PROBE_STRIDE {return;}
    let b=FILM_PROBE_HEADER+n*FILM_PROBE_ENTRY;
    film_probe_put(probe,b,u32(s.x));
    film_probe_put(probe,b+1u,u32(s.y));
    film_probe_put(probe,b+2u,slot|(u32(listed)<<8u)|(u32(valid)<<9u)|(u32(discontinuity)<<10u));
    film_probe_put(probe,b+3u,count);
    film_probe_put(probe,b+4u,r.x);
    film_probe_put(probe,b+5u,r.y);
    film_probe_put(probe,b+6u,r.z);
    film_probe_put(probe,b+7u,r.w);
    film_probe_v3(probe,b+8u,moved);
    let placement=film_placement(r);
    var at=vec3<f32>(0.0);
    if placement!=0u {at=film_world(moved.xy,moved.z);}
    film_probe_v3(probe,b+11u,at);
    // Plane tests in the production order: own, up, left, right, down.
    var mask=0u;
    let side=(r.w&FILM_SIDE)!=0u;
    let e=film_motion.eye.xyz;
    for(var k=0u;k<5u;k+=1u) {
        let j=select(select(select(select(7u,5u,k==3u),3u,k==2u),1u,k==1u),4u,k==0u);
        let q=(*planes)[j];
        if placement!=0u && q.placement!=0u && q.placement!=0xffffffffu {
            if film_on_plane(q,placement,at,side) {mask|=1u<<k;}
            film_probe_f(probe,b+15u+2u*k,abs(film_fdot(q.normal,at-q.world)));
            film_probe_f(probe,b+16u+2u*k,4.0*film_motion.eye.w*abs(film_fdot(q.normal,at-e)));
        }
    }
    film_probe_put(probe,b+14u,mask);
    var occluded=2u;
    if discontinuity {occluded=u32(film_occluded(moved.xy,select(moved.z,0.0,placement==0u)));}
    film_probe_put(probe,b+25u,occluded);
    let near=film_world(moved.xy,1.0);
    let end=film_world(moved.xy,select(moved.z,film_motion.background.w,placement==0u));
    let delta=end-near;
    let length=film_flength(delta);
    let direction=delta/length;
    film_probe_trace(probe,b+26u,near,direction,0.0,length*1.01);
    film_probe_f(probe,b+33u,length);
    if placement!=0u {
        film_probe_trace(probe,b+34u,near,direction,length*(1.0-1.0/8192.0),length*(1.0+1.0/8192.0));
    }
}
fn film_probe_final(probe:u32,v:ptr<function,FilmValidation>) {
    film_probe_put(probe,14u,(*v).count);
    for(var k=0u;k<FILM_MOVED_MAX;k+=1u) {
        if k>=(*v).count {break;}
        let r=(*v).kept[k];
        film_probe_put(probe,128u+4u*k,r.x);
        film_probe_put(probe,129u+4u*k,r.y);
        film_probe_put(probe,130u+4u*k,r.z);
        film_probe_put(probe,131u+4u*k,r.w);
    }
}
