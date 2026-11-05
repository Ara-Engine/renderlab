// ---- T2 world-anchored sample records: motion trace, validation, reset and seed passes ----
// Appended to the motion kernel module only (film_records.rs kernel_source), after the T1
// sequence and the shared record encoding. A moving frame runs:
//   1. film_gap: each pixel gathers the records that now land in its footprint and chooses the
//      next lattice position farthest from them, or the centre when none land; then the motion
//      trace (the fused Interactive trace, mode 0) traces that sample and publishes its plane;
//   2. film_validate: each pixel gathers the same records again and keeps those that pass the
//      geometric tests against this frame's fresh planes, with exact camera rays where the
//      3x3 fresh hits are discontinuous;
//   3. the motion trace again (mode 1): pixels left without a record whose sample was
//      jittered are traced at the centre with index 0, so their image is Interactive's;
//   4. the program's motion shading (film_records_store.wgsl): the mean of the pixel's records
//      and its fresh sample, which becomes slot 0.
// Static frames keep T1a's cumulative mean; film_seed starts it from the records when motion
// stops, and film_records_init starts the records on reset frames.

// Frame state written by the CPU (film_records.rs prepare_records), one buffer per trace mode.
// It is storage, not a uniform (docs/m6-interactive-climb-2.md). The matrices put the far plane
// at infinity (Camera::infinite_matrix), so background records move as directions.
//   reproject: this view-projection times the previous one's inverse, composed in f64;
//   current, current_inverse: this frame's view-projection and its inverse;
//   eye: this frame's eye, w = radians per pixel at the image centre;
//   background: the colour background records contribute (the shading's clear colour), and in
//     w the camera's far plane depth, where camera rays end;
//   size: width, height, trace mode (0 the motion trace, 1 the centre retrace), and the
//     implied-record frame index;
//   flags: x = test diagnostics (film_diagnostics).
struct FilmMotion {
    reproject:mat4x4<f32>,
    current:mat4x4<f32>,
    current_inverse:mat4x4<f32>,
    eye:vec4<f32>,
    background:vec4<f32>,
    size:vec4<u32>,
    flags:vec4<u32>,
};
@group(0) @binding(31) var<storage,read> film_motion:FilmMotion;
// This frame's fresh planes, one per pixel (film_plane).
@group(0) @binding(32) var<storage,read_write> film_planes:array<vec4<u32>>;
// The previous moving frame's records: fresh (slot 0) and moved (slots 1..K-1). The centre
// retrace binds this frame's moved records here instead, for their cold bits.
@group(0) @binding(33) var film_fresh_in:texture_2d<u32>;
@group(0) @binding(34) var film_moved_in:texture_2d_array<u32>;

// Every history decision uses explicit multiply-adds: long-lived pipelines may not
// re-contract them, so replays are bit-identical (docs/determinism.md).
fn film_fdot(a:vec3<f32>,b:vec3<f32>)->f32 {
    return fma(a.x,b.x,fma(a.y,b.y,a.z*b.z));
}
fn film_flength(v:vec3<f32>)->f32 {
    return sqrt(film_fdot(v,v));
}
fn film_apply(m:mat4x4<f32>,v:vec3<f32>)->vec4<f32> {
    return fma(m[0],vec4<f32>(v.x),fma(m[1],vec4<f32>(v.y),fma(m[2],vec4<f32>(v.z),m[3])));
}
// Normalized device x, y of a continuous pixel position (pixel centres at + 0.5), as
// primary_ray maps them.
fn film_ndc(q:vec2<f32>)->vec2<f32> {
    return fma(q/vec2<f32>(film_motion.size.xy),vec2<f32>(2.0,-2.0),vec2<f32>(-1.0,1.0));
}
// A point of the previous frame (continuous pixel position, record depth) in this frame's
// pixel grid: x, y, and its depth.
fn film_reproject(q:vec2<f32>,depth:f32)->vec3<f32> {
    let c=film_apply(film_motion.reproject,vec3<f32>(film_ndc(q),depth));
    let n=c.xyz/c.w;
    let centre=0.5*vec2<f32>(film_motion.size.xy);
    return vec3<f32>(fma(n.xy,centre*vec2<f32>(1.0,-1.0),centre),n.z);
}
// The world point at a continuous pixel position and record depth (not 0) of this frame.
fn film_world(q:vec2<f32>,depth:f32)->vec3<f32> {
    let c=film_apply(film_motion.current_inverse,vec3<f32>(film_ndc(q),depth));
    return c.xyz/c.w;
}

// Octahedral unit normal in two snorm16 halves (precision about 3e-5).
fn film_oct(n:vec3<f32>)->u32 {
    let a=n/(abs(n.x)+abs(n.y)+abs(n.z));
    var e=a.xy;
    if a.z<0.0 {e=(vec2<f32>(1.0)-abs(a.yx))*select(vec2<f32>(-1.0),vec2<f32>(1.0),a.xy>=vec2<f32>(0.0));}
    return pack2x16snorm(e);
}
fn film_unoct(v:u32)->vec3<f32> {
    let e=unpack2x16snorm(v);
    var n=vec3<f32>(e,1.0-abs(e.x)-abs(e.y));
    let t=max(-n.z,0.0);
    n=vec3<f32>(n.x+select(t,-t,n.x>=0.0),n.y+select(t,-t,n.y>=0.0),n.z);
    return n/film_flength(n);
}

// Whether `pixel`'s trace tile kept explicit records in the previous moving frame.
fn film_explicit(pixel:vec2<i32>)->bool {
    let first=vec2<i32>((vec2<u32>(pixel)/PRIMARY_TILE)*PRIMARY_TILE);
    return (textureLoad(film_moved_in,first,0,0).w&FILM_TILE_BIT)!=0u;
}
// Slot `slot` of a pixel in a tile without explicit records: every sample it has seen since
// its tile last kept records was background, and background is uniform, so its records are
// background samples at its own lattice positions of the implied frame index, slot 0 the
// youngest.
fn film_implied(pixel:vec2<i32>,slot:u32)->vec4<u32> {
    let k=(film_motion.size.w-slot)&0xffu;
    return vec4<u32>(0u,0u,0u,film_quantize(film_offset(k,vec2<u32>(pixel),film_motion.size.x))|(slot<<24u)|FILM_VALID);
}
// Previous-frame record `slot` of `pixel`, in the pixel's previous grid.
fn film_source(pixel:vec2<i32>,slot:u32,listed:bool)->vec4<u32> {
    if !listed {return film_implied(pixel,slot);}
    if slot==0u {return textureLoad(film_fresh_in,pixel,0);}
    return textureLoad(film_moved_in,pixel,slot-1u,0);
}
fn film_inside(q:vec2<i32>)->bool {
    return all(q>=vec2<i32>(0)) && all(q<vec2<i32>(film_motion.size.xy));
}
// The direction motion of a pixel centre: implied background records move with it (one
// projection per source; their exact projections differ by about 1e-6 px on a uniform
// background).
fn film_drift(s:vec2<i32>)->vec2<f32> {
    let centre=vec2<f32>(s)+vec2<f32>(0.5);
    return film_reproject(centre,0.0).xy-centre;
}
// Where previous record `r` of source pixel `s` lands in this frame's grid: x, y and depth.
fn film_land(s:vec2<i32>,r:vec4<u32>,listed:bool,drift:vec2<f32>)->vec3<f32> {
    let start=vec2<f32>(s)+film_position(r.w);
    if !listed {return vec3<f32>(start+drift,0.0);}
    return film_reproject(start,bitcast<f32>(r.z));
}

// The previous pixels whose records can land in `p`, as [x0, y0, x1, y1] inclusive. Records
// move with their surfaces, so the window follows the motion of the previous fresh samples
// around p: 3x3 around p's own source when the 3x3 motions agree within half a pixel, else the
// box over every motion, at most 5x5 around p's own source (silhouettes, depth steps).
fn film_source_window(p:vec2<i32>)->vec4<i32> {
    let centre=vec2<f32>(p)+vec2<f32>(0.5);
    var lo=vec2<f32>(3.0e38);
    var hi=vec2<f32>(-3.0e38);
    var own=centre;
    for(var j=0;j<9;j+=1) {
        let q=p+vec2<i32>(j%3-1,j/3-1);
        if !film_inside(q) {continue;}
        let r=film_source(q,0u,film_explicit(q));
        let start=vec2<f32>(q)+film_position(r.w);
        let source=centre-(film_reproject(start,bitcast<f32>(r.z)).xy-start);
        lo=min(lo,source);
        hi=max(hi,source);
        if j==4 {own=source;}
    }
    let base=floor(own);
    if all(hi-own<=vec2<f32>(0.5)) && all(own-lo<=vec2<f32>(0.5)) {
        return vec4<i32>(vec2<i32>(base)-1,vec2<i32>(base)+1);
    }
    let a=clamp(floor(lo)-1.0,base-2.0,base+2.0);
    let b=clamp(floor(hi)+1.0,base-2.0,base+2.0);
    return vec4<i32>(vec2<i32>(a),vec2<i32>(b));
}

// ---- 1 and 3: the motion trace (FILM_MOTION_* seams on the Interactive trace) ----
// Pixels the centre retrace leaves alone.
var<private> film_skip:bool=false;
// This frame's sequence index at the pixel, and its hit's geometric normal (the receiver's).
var<private> film_now:u32=0u;
var<private> film_geometric:vec3<f32>=vec3<f32>(0.0);

// Selects the pixel's sample before it is traced. Mode 0: the index film_gap chose. Mode 1:
// index 0 for cold pixels, nothing for others.
fn film_motion_begin(id:vec2<u32>,size:vec2<u32>) {
    let p=vec2<i32>(id);
    if film_motion.size.z!=0u {
        // Pixels of tiles without explicit records hold stale slots; cold pixels' tiles keep them.
        let r=textureLoad(film_moved_in,p,0,0);
        film_skip=!film_explicit(p) || (r.w&(FILM_COLD|FILM_VALID))!=FILM_COLD;
        film_now=0u;
        film_begin(0u,id,size.x);
        return;
    }
    film_now=film_plane_index(film_planes[id.x+id.y*size.x]);
    film_begin(film_now,id,size.x);
}
// The pixel's sequence index for this moving frame: among the next 16 after the previous one,
// the position farthest from the records landing in the pixel (Euclidean, within the pixel);
// the centre, index 0, when none land.
fn film_gap_index(id:vec2<u32>,size:vec2<u32>)->u32 {
    let p=vec2<i32>(id);
    let previous=film_plane_index(film_planes[id.x+id.y*size.x]);
    // Outside the scene rectangle no ray is traced: the sample is background wherever it lies.
    let scene=primary_frame.scene;
    if any(id<scene.xy) || any(id>=scene.zw) {return (previous+1u)&0xffu;}
    var candidates:array<vec2<f32>,16>;
    var nearest:array<f32,16>;
    for(var c=0u;c<16u;c+=1u) {
        candidates[c]=film_offset((previous+1u+c)&0xffu,id,size.x);
        nearest[c]=4.0;
    }
    var arrived=false;
    let window=film_source_window(p);
    for(var y=window.y;y<=window.w;y+=1) {
        for(var x=window.x;x<=window.z;x+=1) {
            let s=vec2<i32>(x,y);
            if !film_inside(s) {continue;}
            let listed=film_explicit(s);
            var drift=vec2<f32>(0.0);
            if !listed {drift=film_drift(s);}
            for(var slot=0u;slot<FILM_RECORDS;slot+=1u) {
                let r=film_source(s,slot,listed);
                // Valid moved records come first.
                if (r.w&FILM_VALID)==0u {break;}
                let q=film_land(s,r,listed,drift).xy;
                if any(floor(q)!=vec2<f32>(p)) {continue;}
                arrived=true;
                let at=q-vec2<f32>(p);
                for(var c=0u;c<16u;c+=1u) {
                    let d=candidates[c]-at;
                    nearest[c]=min(nearest[c],fma(d.x,d.x,d.y*d.y));
                }
            }
        }
    }
    var k=0u;
    if arrived {
        var best=-1.0;
        for(var c=0u;c<16u;c+=1u) {
            if nearest[c]>best {best=nearest[c];k=(previous+1u+c)&0xffu;}
        }
    }
    return k;
}
// Chooses every pixel's sample before the motion trace, replacing its previous plane with the
// chosen index (the trace then publishes the whole plane).
@compute @workgroup_size(8,4) fn film_gap(@builtin(global_invocation_id) gid:vec3<u32>) {
    let size=film_motion.size.xy;
    if any(gid.xy>=size) {return;}
    film_planes[gid.x+gid.y*size.x]=vec4<u32>(0u,0u,film_gap_index(gid.xy,size)<<24u,0u);
}
// The trace's tile coverage flag. The centre retrace only ever sets it.
fn film_tile(tile:vec2<u32>,covered:u32) {
    if film_motion.size.z==0u || covered!=0u {textureStore(primary_tile_output,tile,vec4<u32>(covered));}
}
// A pixel the centre retrace leaves alone in a tile where a retraced sample hit. The tile may
// have had no hit in the motion trace, which then wrote no hit records there, so a pixel whose
// fresh sample missed stores its miss now; the shading reads every hit record of a covered tile.
fn film_retrace_tile(p:vec2<i32>,covered:u32) {
    if film_motion.size.z==0u || covered==0u {return;}
    if film_plane_placement(film_planes[u32(p.x)+u32(p.y)*film_motion.size.x])!=0u {return;}
    textureStore(primary_hit_output,p,vec4<u32>(0u));
    textureStore(fresh_output,p,vec4<u32>(256u));
}
// Publishes the pixel's fresh plane: depth, geometric normal, placement, side and index.
// With diagnostics, w marks a receiver-band suspect (FILM_BAND): `visible` (of 8 rays) is zero
// and the hit lies within 1e-4 barycentric of a triangle edge.
fn film_plane(p:vec2<i32>,trace:PrimaryTrace,visible:u32) {
    var plane=vec4<u32>(0u,0u,film_now<<24u,0u);
    if trace.record.x!=0u {
        let c=film_apply(film_motion.current,trace.point);
        let side=film_fdot(film_geometric,film_motion.eye.xyz-trace.point)>=0.0;
        let b=bitcast<vec2<f32>>(trace.record.zw);
        let band=film_motion.flags.x!=0u && visible==0u && min(min(b.x,b.y),1.0-b.x-b.y)<1e-4;
        plane=vec4<u32>(bitcast<u32>(c.z/c.w),film_oct(film_geometric),
            (trace.record.x&0x7fffffu)|(u32(side)<<23u)|(film_now<<24u),u32(band));
    }
    film_planes[u32(p.x)+u32(p.y)*film_motion.size.x]=plane;
}

// ---- 2: validation ----
@group(0) @binding(35) var film_moved_out:texture_storage_2d_array<rgba32uint,write>;
// Test diagnostics per pixel (film_motion.flags.x). x: kept moved records in bits 0..3, cold in
// bit 4, discontinuity in bit 5, explicit tile in bit 6, a kept receiver-band suspect in bit 7,
// landed records in bits 8..15, kept records an exact camera ray finds hidden in bits 16..23
// (F5's zero-weight check: only kept records that no validation ray tested are traced),
// validation rays in bits 24..31. y: the pixel's own fresh sample is a band suspect in bit 0,
// kept band suspects in bits 8..15.
@group(0) @binding(36) var<storage,read_write> film_diagnostics:array<vec2<u32>>;
var<workgroup> film_tile_keeps:atomic<u32>;

// A fresh plane of this frame, decoded: placement + 1 (0: background, 0xffffffff: outside the
// image), its hit point, normal, side and view distance.
struct FilmPlane { placement:u32, world:vec3<f32>, normal:vec3<f32>, side:bool, distance:f32 };
fn film_decode_plane(q:vec2<i32>)->FilmPlane {
    if !film_inside(q) {return FilmPlane(0xffffffffu,vec3<f32>(0.0),vec3<f32>(0.0),false,0.0);}
    let plane=film_planes[u32(q.x)+u32(q.y)*film_motion.size.x];
    let placement=film_plane_placement(plane);
    if placement==0u {return FilmPlane(0u,vec3<f32>(0.0),vec3<f32>(0.0),false,0.0);}
    let at=vec2<f32>(q)+film_offset(film_plane_index(plane),vec2<u32>(q),film_motion.size.x);
    let world=film_world(at,bitcast<f32>(plane.x));
    return FilmPlane(placement,world,film_unoct(plane.y),film_plane_side(plane),film_flength(world-film_motion.eye.xyz));
}
// A geometry record at world point `x` lies on `plane`'s surface: the same placement (and side,
// for thin materials) and, along this frame's camera ray through x, within four pixel
// footprints of the plane: |t - t_plane| <= 4 a t, written without the division.
fn film_on_plane(plane:FilmPlane,placement:u32,x:vec3<f32>,side:bool)->bool {
    if plane.placement!=placement {return false;}
    if (primary_placements[placement-1u]&0x20000000u)!=0u && plane.side!=side {return false;}
    let e=film_motion.eye.xyz;
    return abs(film_fdot(plane.normal,x-plane.world))<=4.0*film_motion.eye.w*abs(film_fdot(plane.normal,x-e));
}
// An exact camera any-hit ray from the near plane through `q` to the record's point at `depth`
// (for background, to the far plane, as camera rays) finds geometry first. Geometry records
// stop 2^-12 short of their own point. Traced frames with hardware shadows hold double-sided
// draws only, so no facing test is needed.
fn film_occluded(q:vec2<f32>,depth:f32)->bool {
    let near=film_world(q,1.0);
    let delta=film_world(q,select(depth,film_motion.background.w,depth==0.0))-near;
    let length=film_flength(delta);
    var tmax=length;
    if depth!=0.0 {tmax=length*(1.0-1.0/4096.0);}
    var query:ray_query;
    rayQueryInitialize(&query,fresh_scene,RayDesc(RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_TERMINATE_ON_FIRST_HIT,255u,0.0,tmax,near,delta/length));
    while rayQueryProceed(&query) {}
    return rayQueryGetCommittedIntersection(&query).kind!=0u;
}

// Each pixel gathers the previous records landing in its footprint, in a fixed order, and keeps
// up to K - 1 that pass validation, youngest first (equal ages: the earlier in gather order):
// - geometry: on the pixel's own fresh plane or a 4-neighbour's (film_on_plane);
// - background: when the 3x3 fresh samples are all background;
// - in discontinuity pixels, whose 3x3 fresh samples mix geometry and background or span more
//   than four footprints in distance, every record also needs an unoccluded exact camera ray
//   (film_occluded), which is the only way background records are kept there.
// A tile keeps explicit records when any pixel has fresh geometry around it, keeps a geometry
// record, or is cold. Records only move between frames, so their colour is copied unchanged.
struct FilmValidation { kept:array<vec4<u32>,FILM_MOVED_MAX>, count:u32, keeps:bool, cold:bool, report:vec2<u32> };
fn film_validate_pixel(p:vec2<i32>)->FilmValidation {
    var v:FilmValidation;
    let i=u32(p.x)+u32(p.y)*film_motion.size.x;
    var planes:array<FilmPlane,9>;
    var geometry=0u;
    var background=0u;
    var near=3.0e38;
    var far=0.0;
    for(var j=0;j<9;j+=1) {
        planes[j]=film_decode_plane(p+vec2<i32>(j%3-1,j/3-1));
        if planes[j].placement==0u {background+=1u;}
        else if planes[j].placement!=0xffffffffu {
            geometry+=1u;
            near=min(near,planes[j].distance);
            far=max(far,planes[j].distance);
        }
    }
    let discontinuity=(geometry>0u && background>0u) || far-near>4.0*film_motion.eye.w*near;
    var landed=0u;
    var rays=0u;
    var geometric=false;
    let window=film_source_window(p);
    for(var y=window.y;y<=window.w;y+=1) {
        for(var x=window.x;x<=window.z;x+=1) {
            let s=vec2<i32>(x,y);
            if !film_inside(s) {continue;}
            let listed=film_explicit(s);
            var drift=vec2<f32>(0.0);
            if !listed {drift=film_drift(s);}
            for(var slot=0u;slot<FILM_RECORDS;slot+=1u) {
                let r=film_source(s,slot,listed);
                if (r.w&FILM_VALID)==0u {break;}
                let moved=film_land(s,r,listed,drift);
                if any(floor(moved.xy)!=vec2<f32>(p)) {continue;}
                landed+=1u;
                let placement=film_placement(r);
                var valid=false;
                if placement==0u {
                    valid=geometry==0u;
                    if discontinuity {rays+=1u;valid=!film_occluded(moved.xy,0.0);}
                } else {
                    let side=(r.w&FILM_SIDE)!=0u;
                    let at=film_world(moved.xy,moved.z);
                    valid=film_on_plane(planes[4],placement,at,side) || film_on_plane(planes[1],placement,at,side)
                        || film_on_plane(planes[3],placement,at,side) || film_on_plane(planes[5],placement,at,side)
                        || film_on_plane(planes[7],placement,at,side);
                    if valid && discontinuity {rays+=1u;valid=!film_occluded(moved.xy,moved.z);}
                }
                if !valid {continue;}
                let record=vec4<u32>(r.xy,bitcast<u32>(moved.z),film_quantize(moved.xy-vec2<f32>(p))
                    |(min(film_age(r.w)+1u,FILM_AGE_MAX)<<24u)|(r.w&(FILM_SIDE|FILM_BAND))|FILM_VALID);
                if v.count+1u<FILM_RECORDS {
                    v.kept[v.count]=record;
                    v.count+=1u;
                } else {
                    // Replace the oldest (the last of equals) only by a strictly younger record.
                    var worst=0u;
                    for(var k=1u;k<v.count;k+=1u) {
                        if film_age(v.kept[k].w)>=film_age(v.kept[worst].w) {worst=k;}
                    }
                    if film_age(record.w)<film_age(v.kept[worst].w) {v.kept[worst]=record;}
                }
            }
        }
    }
    for(var k=0u;k<v.count;k+=1u) {geometric=geometric || film_placement(v.kept[k])!=0u;}
    // Outside the scene rectangle no ray is traced, so every index gives Interactive's image.
    let scene=primary_frame.scene;
    v.cold=v.count==0u && film_plane_index(film_planes[i])!=0u
        && all(vec2<u32>(p)>=scene.xy) && all(vec2<u32>(p)<scene.zw);
    v.keeps=geometry>0u || geometric || v.cold;
    if film_motion.flags.x!=0u {
        var hidden=0u;
        var band=0u;
        for(var k=0u;k<v.count;k+=1u) {
            band+=u32((v.kept[k].w&FILM_BAND)!=0u);
            if !discontinuity {
                let at=vec2<f32>(p)+film_position(v.kept[k].w);
                rays+=1u;
                hidden+=u32(film_occluded(at,bitcast<f32>(v.kept[k].z)));
            }
        }
        v.report=vec2<u32>(v.count|(u32(v.cold)<<4u)|(u32(discontinuity)<<5u)|(u32(band>0u)<<7u)
            |(min(landed,255u)<<8u)|(min(hidden,255u)<<16u)|(min(rays,255u)<<24u),(film_planes[i].w&1u)|(band<<8u));
    }
    return v;
}
@compute @workgroup_size(8,4) fn film_validate(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(workgroup_id) group:vec3<u32>) {
    let size=film_motion.size.xy;
    let inside=all(gid.xy<size);
    let p=vec2<i32>(gid.xy);
    // A pixel whose 3x3 lies wholly outside the scene rectangle sees only background around it:
    // it can keep no geometry record and is never cold, so it gathers only when another pixel
    // makes its tile keep explicit records. Otherwise it holds implied background records.
    let scene=primary_frame.scene;
    let outside=any(gid.xy+vec2<u32>(1u)<scene.xy) || any(gid.xy>=scene.zw+vec2<u32>(1u));
    var v:FilmValidation;
    var gathered=false;
    if inside && !outside {
        v=film_validate_pixel(p);
        gathered=true;
    }
    if v.keeps {atomicOr(&film_tile_keeps,1u);}
    workgroupBarrier();
    let listed=atomicLoad(&film_tile_keeps)!=0u;
    if !inside {return;}
    if listed && !gathered {v=film_validate_pixel(p);}
    let first=all(gid.xy==group.xy*PRIMARY_TILE);
    if listed {
        for(var s=0u;s+1u<FILM_RECORDS;s+=1u) {
            var r=vec4<u32>(0u);
            if s<v.count {r=v.kept[s];} else if s==0u && v.cold {r.w=FILM_COLD;}
            if s==0u && first {r.w|=FILM_TILE_BIT;}
            textureStore(film_moved_out,p,s,r);
        }
    } else if first {
        textureStore(film_moved_out,p,0u,vec4<u32>(0u));
    }
    if film_motion.flags.x!=0u {
        // Pixels left implied report their implied background records.
        if !gathered && !listed {v.report=vec2<u32>(FILM_RECORDS-1u,0u);}
        film_diagnostics[gid.x+gid.y*size.x]=v.report|vec2<u32>(u32(listed)<<6u,0u);
    }
}

// ---- Reset frames: the records start from the Interactive image ----
// After the production Interactive frame and film_init: every covered tile keeps explicit
// records, each pixel's fresh record is its Interactive sample at the centre (index 0) and its
// moved slots are empty. Every pixel publishes its plane, which the next moving frame reads for
// its previous sequence index.
@group(0) @binding(37) var film_fresh_out:texture_storage_2d<rgba32uint,write>;
@group(0) @binding(38) var film_hdr_now:texture_2d<f32>;
struct FilmRecordVec4s { data:array<vec4<f32>> };
struct FilmRecordU32s { data:array<u32> };
@group(1) @binding(2) var<storage,read> film_record_vertices:binding_array<FilmRecordVec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> film_record_indices:binding_array<FilmRecordU32s,PRIMARY_BATCHES>;
fn film_record_position(batch:u32,index:u32)->vec3<f32> {
    return film_record_vertices[batch].data[film_record_indices[batch].data[index]*3u].xyz;
}
@compute @workgroup_size(8,4) fn film_records_init(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(workgroup_id) group:vec3<u32>) {
    let size=film_motion.size.xy;
    if any(gid.xy>=size) {return;}
    let p=vec2<i32>(gid.xy);
    // Uncovered trace tiles keep stale hit records; every pixel there is background.
    let covered=textureLoad(primary_tiles,gid.xy/PRIMARY_TILE,0).x!=0u;
    let centre=2048u|(2048u<<12u);
    var plane=vec4<u32>(0u);
    if covered {
        let record=textureLoad(primary_hit,p,0);
        var r=vec4<u32>(0u,0u,0u,centre|FILM_VALID);
        if record.x!=0u {
            let slot=record.x-1u;
            let batch=primary_placements[slot]&0x1fffffffu;
            let i=record.y*3u;
            let a=film_record_position(batch,i);
            let b=film_record_position(batch,i+1u);
            let c=film_record_position(batch,i+2u);
            let bary=bitcast<vec2<f32>>(record.zw);
            let x=film_apply(models[slot],receiver_point(a,b,c,vec3<f32>(1.0-bary.x-bary.y,bary))).xyz;
            let n=primary_geometric(a,b,c,slot);
            let side=film_fdot(n,film_motion.eye.xyz-x)>=0.0;
            let clip=film_apply(film_motion.current,x);
            let depth=clip.z/clip.w;
            r=film_record(textureLoad(film_hdr_now,p,0).rgb,record.x,depth,centre,side);
            plane=vec4<u32>(bitcast<u32>(depth),film_oct(n),(record.x&0x7fffffu)|(u32(side)<<23u),0u);
        }
        textureStore(film_fresh_out,p,r);
    }
    film_planes[gid.x+gid.y*size.x]=plane;
    let first=all(gid.xy==group.xy*PRIMARY_TILE);
    if covered {
        for(var s=0u;s+1u<FILM_RECORDS;s+=1u) {
            textureStore(film_moved_out,p,s,vec4<u32>(0u,0u,0u,select(0u,FILM_TILE_BIT,s==0u && first)));
        }
    } else if first {
        textureStore(film_moved_out,p,0u,vec4<u32>(0u));
    }
}

// ---- Motion stops: T1a's cumulative mean starts from the records ----
// Before the first static frame after motion (whose camera is the last moving frame's), each
// explicit-record pixel's T1a history becomes the mean of its valid records, fresh first, with
// weight the record count, and its anchor the fresh sample's placement and index. Tiles without
// explicit records keep T1a's implied background history.
@group(0) @binding(39) var film_seed_colour:texture_storage_2d<rgba32float,write>;
@group(0) @binding(40) var film_seed_anchor:texture_storage_2d<rg32uint,write>;
@group(0) @binding(41) var film_seed_tiles:texture_storage_2d<r32uint,write>;
@compute @workgroup_size(8,4) fn film_seed(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(workgroup_id) group:vec3<u32>) {
    let size=film_motion.size.xy;
    if any(gid.xy>=size) {return;}
    let p=vec2<i32>(gid.xy);
    let listed=film_explicit(p);
    if all(gid.xy==group.xy*PRIMARY_TILE) {textureStore(film_seed_tiles,group.xy,vec4<u32>(u32(listed)));}
    if !listed {return;}
    let fresh=textureLoad(film_fresh_in,p,0);
    let background=film_motion.background.rgb;
    var sum=film_colour(fresh,background);
    var n=1.0;
    for(var s=0u;s+1u<FILM_RECORDS;s+=1u) {
        let r=textureLoad(film_moved_in,p,s,0);
        if (r.w&FILM_VALID)==0u {break;}
        sum+=film_colour(r,background);
        n+=1.0;
    }
    textureStore(film_seed_colour,p,vec4<f32>(sum/n,n));
    let k=film_plane_index(film_planes[gid.x+gid.y*size.x]);
    textureStore(film_seed_anchor,p,vec4<u32>((film_placement(fresh)&0x7fffffu)|(k<<24u),0u,0u,0u));
}
// ---- end of the T2 kernel passes ----
