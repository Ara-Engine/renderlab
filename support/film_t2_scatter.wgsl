// Test-only scatter variant of T2a's two record gathers (src/film_records.rs ScatterPrototype),
// appended to the motion kernel module. A moving frame runs film_scatter, film_scatter_gap,
// the motion trace, film_scatter_validate, the centre retrace and the motion shading:
// - film_scatter reprojects each previous record once (fresh and moved, explicit or implied) and
//   appends its gather-order key, source pixel index x K + slot, to the pixel it lands in;
// - the gap choice and validation read only those keys, keep those whose source lies in the
//   pixel's production source window, and recompute each landing with the production code.
// Validation sorts the keys into gather order (source raster order, then slot) and then keeps
// records exactly as film_validate_pixel does, so the output never depends on arrival order and
// equals the gather's whenever both see the same landings. A pixel with more arrivals than its
// capacity (2K) gathers as in production instead.
@group(0) @binding(43) var<storage,read_write> film_scatter_counts:array<atomic<u32>>;
@group(0) @binding(44) var<storage,read_write> film_scatter_keys:array<u32>;
const FILM_SCATTER_MAX:u32=16u;
fn film_scatter_capacity()->u32 {
    return min(2u*FILM_RECORDS,FILM_SCATTER_MAX);
}

@compute @workgroup_size(8,4) fn film_scatter(@builtin(global_invocation_id) gid:vec3<u32>) {
    let size=film_motion.size.xy;
    if any(gid.xy>=size) {return;}
    let s=vec2<i32>(gid.xy);
    let listed=film_explicit(s);
    var drift=vec2<f32>(0.0);
    if !listed {drift=film_drift(s);}
    let base=(gid.x+gid.y*size.x)*FILM_RECORDS;
    let capacity=film_scatter_capacity();
    for(var slot=0u;slot<FILM_RECORDS;slot+=1u) {
        let r=film_source(s,slot,listed);
        if (r.w&FILM_VALID)==0u {break;}
        let q=floor(film_land(s,r,listed,drift).xy);
        // Also false for NaN landings.
        if !(all(q>=vec2<f32>(0.0)) && all(q<vec2<f32>(size))) {continue;}
        let i=u32(q.x)+u32(q.y)*size.x;
        let n=atomicAdd(&film_scatter_counts[i],1u);
        if n<capacity {film_scatter_keys[i*capacity+n]=base+slot;}
    }
}

// The pixel's arrivals in gather order (keys ascending), and their count; more than the capacity
// returns the count only.
struct FilmArrivals { keys:array<u32,FILM_SCATTER_MAX>, count:u32 };
fn film_scatter_arrivals(i:u32,sorted:bool)->FilmArrivals {
    var a:FilmArrivals;
    a.count=atomicLoad(&film_scatter_counts[i]);
    let capacity=film_scatter_capacity();
    if a.count>capacity {return a;}
    for(var j=0u;j<a.count;j+=1u) {
        let key=film_scatter_keys[i*capacity+j];
        var k=j;
        if sorted {
            while k>0u && a.keys[k-1u]>key {
                a.keys[k]=a.keys[k-1u];
                k-=1u;
            }
        }
        a.keys[k]=key;
    }
    return a;
}
fn film_scatter_source(key:u32)->vec2<i32> {
    let s=key/FILM_RECORDS;
    return vec2<i32>(i32(s%film_motion.size.x),i32(s/film_motion.size.x));
}
fn film_in_window(window:vec4<i32>,s:vec2<i32>)->bool {
    return all(s>=window.xy) && all(s<=window.zw);
}

// film_gap_index over the pixel's arrivals. The nearest-distance minima do not depend on order.
fn film_scatter_gap_index(id:vec2<u32>,size:vec2<u32>,a:FilmArrivals)->u32 {
    let p=vec2<i32>(id);
    let previous=film_plane_index(film_planes[id.x+id.y*size.x]);
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
    for(var j=0u;j<a.count;j+=1u) {
        let s=film_scatter_source(a.keys[j]);
        if !film_in_window(window,s) {continue;}
        let slot=a.keys[j]%FILM_RECORDS;
        let listed=film_explicit(s);
        var drift=vec2<f32>(0.0);
        if !listed {drift=film_drift(s);}
        let q=film_land(s,film_source(s,slot,listed),listed,drift).xy;
        if any(floor(q)!=vec2<f32>(p)) {continue;}
        arrived=true;
        let at=q-vec2<f32>(p);
        for(var c=0u;c<16u;c+=1u) {
            let d=candidates[c]-at;
            nearest[c]=min(nearest[c],fma(d.x,d.x,d.y*d.y));
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
@compute @workgroup_size(8,4) fn film_scatter_gap(@builtin(global_invocation_id) gid:vec3<u32>) {
    let size=film_motion.size.xy;
    if any(gid.xy>=size) {return;}
    let i=gid.x+gid.y*size.x;
    let a=film_scatter_arrivals(i,false);
    var k:u32;
    if a.count>film_scatter_capacity() {k=film_gap_index(gid.xy,size);}
    else {k=film_scatter_gap_index(gid.xy,size,a);}
    film_planes[i]=vec4<u32>(0u,0u,k<<24u,0u);
}

// film_validate_pixel over the pixel's arrivals in gather order; its per-record body unchanged.
fn film_scatter_validate_pixel(p:vec2<i32>)->FilmValidation {
    let i=u32(p.x)+u32(p.y)*film_motion.size.x;
    let a=film_scatter_arrivals(i,true);
    if a.count>film_scatter_capacity() {return film_validate_pixel(p);}
    var v:FilmValidation;
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
    for(var j=0u;j<a.count;j+=1u) {
        let s=film_scatter_source(a.keys[j]);
        if !film_in_window(window,s) {continue;}
        let slot=a.keys[j]%FILM_RECORDS;
        let listed=film_explicit(s);
        var drift=vec2<f32>(0.0);
        if !listed {drift=film_drift(s);}
        let r=film_source(s,slot,listed);
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
            var worst=0u;
            for(var k=1u;k<v.count;k+=1u) {
                if film_age(v.kept[k].w)>=film_age(v.kept[worst].w) {worst=k;}
            }
            if film_age(record.w)<film_age(v.kept[worst].w) {v.kept[worst]=record;}
        }
    }
    for(var k=0u;k<v.count;k+=1u) {geometric=geometric || film_placement(v.kept[k])!=0u;}
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
// film_validate with the scatter's validation.
@compute @workgroup_size(8,4) fn film_scatter_validate(@builtin(global_invocation_id) gid:vec3<u32>,@builtin(workgroup_id) group:vec3<u32>) {
    let size=film_motion.size.xy;
    let inside=all(gid.xy<size);
    let p=vec2<i32>(gid.xy);
    let scene=primary_frame.scene;
    let outside=any(gid.xy+vec2<u32>(1u)<scene.xy) || any(gid.xy>=scene.zw+vec2<u32>(1u));
    var v:FilmValidation;
    var gathered=false;
    if inside && !outside {
        v=film_scatter_validate_pixel(p);
        gathered=true;
    }
    if v.keeps {atomicOr(&film_tile_keeps,1u);}
    workgroupBarrier();
    let listed=atomicLoad(&film_tile_keeps)!=0u;
    if !inside {return;}
    if listed && !gathered {v=film_scatter_validate_pixel(p);}
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
        if !gathered && !listed {v.report=vec2<u32>(FILM_RECORDS-1u,0u);}
        film_diagnostics[gid.x+gid.y*size.x]=v.report|vec2<u32>(u32(listed)<<6u,0u);
    }
    // The next moving frame's scatter starts from zero arrivals.
    atomicStore(&film_scatter_counts[gid.x+gid.y*size.x],0u);
}
