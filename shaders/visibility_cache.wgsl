// World-space sun-visibility cache for non-RT shadows: unit N7a of docs/non-rt-n7.md.
// Test-only. src/visibility_cache.rs appends this file to N0's traced module
// (shaders/software_bvh.wgsl) and keeps the CPU mirror of its keys, lattice and window.
//
// Cells. A receiver triangle at level k is split into the 4^k sub-triangles of its n = 2^k
// barycentric subdivision; a cell is one sub-triangle, keyed by (placement, primitive,
// level, i, j, upper, side) and sampled at its centroid, so cell values are independent of
// the camera. A pixel picks k old its footprint (cell area about one pixel at normal
// incidence) and reads the cell under its hit barycentrics.
//
// Directions. Each cell owns a jittered, rotated square lattice in a gnomonic chart fixed in
// the world around the anchor sun direction. The lattice points inside the current sun disk
// are the cell's rays (8 on average). A cell stores a 4x4 window of lattice points that
// covers the disk: 16 blocked bits, 16 valid bits and the window's origin. When the sun
// moves, the window slides; points that stay keep their bits, and only points entering the
// disk are traced.
//
// Static visibility only. Fills trace against static placements (instance info.w == 0).
// Each frame, lookups test a cell's unblocked rays against the movers' own BVHs. Receivers
// on movers are not cached: they take the Interactive fallback.
//
// Table slots are four words: key a (atomic claim word), key b, bits (blocked | valid << 16)
// and info (enqueue stamp | window i << 16 | window j << 24, signed bytes). A zero-cleared
// table is empty: key a = 0 is never a key and key b = 0 is never visible for a claimed key.

struct CacheParams {
    // xyz: chart axes and centre (the anchor sun); w: lattice spacing, window radius (chart
    // units, tan alpha widened for the chart's distortion at the current sun), unused.
    chart_u:vec4<f32>,chart_v:vec4<f32>,chart_l:vec4<f32>,
    // xyz: current unit direction toward the sun; w: cos alpha. The disk is the cone of
    // half-angle alpha around it, Interactive's estimand.
    sun:vec4<f32>,
    scene_lo:vec4<f32>,scene_hi:vec4<f32>,
    // x: radians per pixel; y: cell scale (footprints per cell edge).
    view:vec4<f32>,
    // x: frame stamp (1..65535); y: slot mask; z: fill budget (cells); w: flags (bit 0 prefetch).
    frame:vec4<u32>,
    // x: queue capacity per region; y: pending capacity; z: probe limit; w: movers present.
    limits:vec4<u32>,
};
struct CacheWords {data:array<atomic<u32>>};
// One-element binding arrays: N0's layout already uses the stage's eight storage buffers.
@group(2) @binding(47) var<storage,read> cache_params:binding_array<CacheParams,1>;
@group(2) @binding(48) var<storage,read_write> cache_table:binding_array<CacheWords,1>;
@group(2) @binding(49) var<storage,read_write> cache_work:binding_array<CacheWords,1>;

// cache_work words: counters, indirect arguments, then the primary queue, the prefetch queue
// and the pending pixel list (pixel, slot pairs). visibility_cache.rs Work must match.
const CACHE_PENDING:u32=0u;
const CACHE_QUEUED:u32=1u;      // primary (visible) requests, then prefetch at 2
const CACHE_FULL_COUNT:u32=3u;  // probe limit reached
const CACHE_BUSY_COUNT:u32=4u;  // claims in flight
const CACHE_FILLED:u32=5u;
const CACHE_RAYS:u32=6u;
const CACHE_FALLBACK:u32=7u;
const CACHE_MOVER_RAYS:u32=8u;
const CACHE_WARM:u32=9u;
const CACHE_INCOMPLETE:u32=10u;
const CACHE_INSERTED:u32=11u;
const CACHE_FILL_PRIMARY:u32=12u;  // cells the plan takes old each region
const CACHE_FILL_PREFETCH:u32=13u;
const CACHE_MISMATCH:u32=14u;      // validation replay
const CACHE_CHECKED:u32=15u;
const CACHE_FILL_ARGS:u32=16u;
const CACHE_RESOLVE_ARGS:u32=20u;
const CACHE_DUMP:u32=24u;          // first mismatches: slot, bit pairs at 24..31
const CACHE_QUEUES:u32=32u;

const CACHE_LOCK:u32=1u;           // key a of a claim in flight (placement field 0)
const CACHE_NONE:u32=0xffffffffu;
// cache_find states.
const CACHE_FOUND:u32=0u;
const CACHE_NEW:u32=1u;
const CACHE_ABSENT:u32=2u;
const CACHE_BUSY:u32=3u;
const CACHE_OVER:u32=4u;
// Finest level: 14 bits of i and j in key b.
const CACHE_MAX_LEVEL:u32=14u;

fn cache_hash(a:u32,b:u32)->u32 {return fresh_hash(a^fresh_hash(b^0x9e3779b9u));}
fn cache_queue(region:u32)->u32 {return CACHE_QUEUES+region*cache_params[0].limits.x;}
fn cache_pending_base()->u32 {return CACHE_QUEUES+2u*cache_params[0].limits.x;}

// Key words. a: primitive (22 bits) | placement + 1 << 22 (6 bits) | level << 28.
// b: 1 + (j | i << 14 | upper << 28 | side << 29).
fn cache_key_a(placement:u32,primitive:u32,level:u32)->u32 {return primitive|((placement+1u)<<22u)|(level<<28u);}
fn cache_key_b(i:u32,j:u32,upper:u32,side:u32)->u32 {return 1u+(j|(i<<14u)|(upper<<28u)|(side<<29u));}

// The cell (i, j, upper) of barycentrics (b1, b2) at subdivision n.
fn cache_cell(bary:vec2<f32>,level:u32)->vec3<u32> {
    let n=f32(1u<<level);let last=(1u<<level)-1u;
    let x=bary*n;
    let i=min(u32(max(floor(x.x),0.0)),last);
    var j=min(u32(max(floor(x.y),0.0)),last);
    var upper=u32((x.x-f32(i))+(x.y-f32(j))>1.0);
    if i+j>last {j=last-i;upper=0u;}
    if i+j==last {upper=0u;}
    return vec3<u32>(i,j,upper);
}
// The level whose cells have about one pixel footprint of area: round(log2(sqrt(A) / f)).
fn cache_level(area:f32,footprint:f32)->u32 {
    let k=round(log2(max(sqrt(area),1e-30)/max(footprint,1e-30)));
    return u32(clamp(k,0.0,f32(CACHE_MAX_LEVEL)));
}

// Find, or claim, the slot of key (a, b) by linear probing. vec2(slot, state).
fn cache_find(a:u32,b:u32,insert:bool)->vec2<u32> {
    let mask=cache_params[0].frame.y;
    var s=cache_hash(a,b)&mask;
    for(var k=0u;k<cache_params[0].limits.z;k+=1u) {
        let w=4u*s;
        var ka=atomicLoad(&cache_table[0].data[w]);
        if ka==0u {
            if !insert {return vec2<u32>(s,CACHE_ABSENT);}
            // Weak exchanges may fail spuriously; retry while the slot still reads empty.
            for(var attempt=0u;attempt<4u && ka==0u;attempt+=1u) {
                let r=atomicCompareExchangeWeak(&cache_table[0].data[w],0u,CACHE_LOCK);
                if r.exchanged {
                    atomicStore(&cache_table[0].data[w+1u],b);
                    atomicStore(&cache_table[0].data[w+2u],0u);
                    atomicStore(&cache_table[0].data[w+3u],cache_params[0].frame.x);
                    atomicStore(&cache_table[0].data[w],a);
                    atomicAdd(&cache_work[0].data[CACHE_INSERTED],1u);
                    return vec2<u32>(s,CACHE_NEW);
                }
                ka=r.old_value;
            }
            if ka==0u {return vec2<u32>(s,CACHE_BUSY);}
        }
        if ka==CACHE_LOCK {return vec2<u32>(s,CACHE_BUSY);}
        if ka==a {
            let kb=atomicLoad(&cache_table[0].data[w+1u]);
            if kb==b {return vec2<u32>(s,CACHE_FOUND);}
            if kb==0u {return vec2<u32>(s,CACHE_BUSY);}
        }
        s=(s+1u)&mask;
    }
    return vec2<u32>(CACHE_NONE,CACHE_OVER);
}

// A cell's direction lattice: offset in [0, 1)^2 and rotation, old its key.
struct CacheLattice {offset:vec2<f32>,c:f32,s:f32};
fn cache_lattice(a:u32,b:u32)->CacheLattice {
    let h1=fresh_hash(a^fresh_hash(b));let h2=fresh_hash(h1^0xa511e9b3u);let h3=fresh_hash(h2^0x63d83595u);
    let offset=vec2<f32>(f32(h1>>8u),f32(h2>>8u))*(1.0/16777216.0);
    let angle=6.283185307179586*f32(h3>>8u)*(1.0/16777216.0);
    return CacheLattice(offset,cos(angle),sin(angle));
}
// The sun's chart point in the cell's lattice coordinates (units of the spacing).
fn cache_centre(t:CacheLattice)->vec2<f32> {
    let p=cache_params[0];
    let l=p.sun.xyz;
    let c=vec2<f32>(dot(l,p.chart_u.xyz),dot(l,p.chart_v.xyz))/(dot(l,p.chart_l.xyz)*p.chart_u.w);
    return vec2<f32>(t.c*c.x+t.s*c.y,-t.s*c.x+t.c*c.y)-t.offset;
}
// Origin of the 4x4 window covering the disk: the first lattice indices at or above
// centre - radius. The disk is at most 3.6 spacings wide (visibility_cache.rs keeps the
// sun near enough to the anchor), so four columns and rows cover it.
fn cache_window(centre:vec2<f32>)->vec2<i32> {
    let r=cache_params[0].chart_v.w/cache_params[0].chart_u.w;
    return vec2<i32>(ceil(centre-vec2<f32>(r)));
}
// World direction of the cell's lattice point at absolute indices q.
fn cache_direction(t:CacheLattice,q:vec2<i32>)->vec3<f32> {
    let p=cache_params[0];
    let x=(vec2<f32>(q)+t.offset)*p.chart_u.w;
    let chart=vec2<f32>(t.c*x.x-t.s*x.y,t.s*x.x+t.c*x.y);
    return normalize(p.chart_l.xyz+chart.x*p.chart_u.xyz+chart.y*p.chart_v.xyz);
}
// Window bits whose directions lie in the sun's cone.
fn cache_disk(t:CacheLattice,window:vec2<i32>)->u32 {
    var bits=0u;
    for(var b=0u;b<16u;b+=1u) {
        let d=cache_direction(t,window+vec2<i32>(i32(b&3u),i32(b>>2u)));
        if dot(d,cache_params[0].sun.xyz)>cache_params[0].sun.w {bits|=1u<<b;}
    }
    return bits;
}
// Window bits (low 16) moved old window `old` to window `to`; points leaving are dropped.
fn cache_shift(bits:u32,old:vec2<i32>,to:vec2<i32>)->u32 {
    let d=to-old;
    if all(d==vec2<i32>(0)) {return bits;}
    var out=0u;
    for(var b=0u;b<16u;b+=1u) {
        if (bits&(1u<<b))==0u {continue;}
        let x=i32(b&3u)-d.x;let y=i32(b>>2u)-d.y;
        if x>=0 && x<4 && y>=0 && y<4 {out|=1u<<(u32(x)+4u*u32(y));}
    }
    return out;
}
fn cache_meta_window(info:u32)->vec2<i32> {
    return vec2<i32>(bitcast<i32>(info<<8u)>>24u,bitcast<i32>(info)>>24u);
}
fn cache_meta(stamp:u32,window:vec2<i32>)->u32 {
    return (stamp&0xffffu)|((u32(window.x)&0xffu)<<16u)|((u32(window.y)&0xffu)<<24u);
}

// The receiver at a cell's centroid: the placement's triangle, the side of the key, ÃŽÂµ.
struct CacheReceiver {world:vec3<f32>,geometric:vec3<f32>,epsilon:f32,thin:bool,side:u32};
fn cache_receiver(a:u32,b:u32)->CacheReceiver {
    let placement=((a>>22u)&63u)-1u;let primitive=a&0x3fffffu;let level=a>>28u;
    let k=b-1u;let j=k&0x3fffu;let i=(k>>14u)&0x3fffu;let upper=(k>>28u)&1u;let side=(k>>29u)&1u;
    let n=f32(1u<<level);
    let corner=select(vec2<f32>(1.0/3.0),vec2<f32>(2.0/3.0),upper!=0u);
    let bary=(vec2<f32>(f32(i),f32(j))+corner)/n;
    let v=software_positions(software_instances[placement].info.y,primitive);
    let w=vec3<f32>(1.0-bary.x-bary.y,bary);
    let world=world_position(receiver_point(v[0],v[1],v[2],w),placement).xyz;
    return CacheReceiver(world,primary_geometric(v[0],v[1],v[2],placement),primary_epsilon(v,placement),
        (primary_placements[placement]&0x20000000u)!=0u,side);
}
// Shadow-ray origin: the key's side for opaque receivers (the viewed side when keyed), the
// ray's side for thin ones, as shadow_origin chooses them.
fn cache_origin(r:CacheReceiver,direction:vec3<f32>)->vec3<f32> {
    var normal=select(r.geometric,-r.geometric,r.side!=0u);
    if r.thin {normal=select(-r.geometric,r.geometric,dot(r.geometric,direction)>=0.0);}
    return fma(normal,vec3<f32>(r.epsilon),r.world);
}
// Where the ray leaves the scene box.
fn cache_tmax(origin:vec3<f32>,direction:vec3<f32>)->f32 {
    let p=cache_params[0];
    let ray=software_ray(origin,direction);
    let a=(p.scene_lo.xyz-origin)*ray.inverse;let b=(p.scene_hi.xyz-origin)*ray.inverse;
    let far=max(a,b);
    return min(min(far.x,far.y),far.z);
}
// N0's opaque any-hit over static placements (movers false) or movers only (true).
fn cache_any(origin:vec3<f32>,direction:vec3<f32>,tmin:f32,tmax:f32,movers:bool)->bool {
    let world=software_ray(origin,direction);
    for(var i=0u;i<SOFTWARE_INSTANCES;i+=1u) {
        let instance=software_instances[i];
        if (instance.info.w!=0u)!=movers {continue;}
        if software_box(world,instance.lo.xyz,instance.hi.xyz,tmin,tmax)>tmax {continue;}
        let m=instance.rows;let p=vec4<f32>(origin,1.0);
        let o=vec3<f32>(dot(m[0],p),dot(m[1],p),dot(m[2],p));
        let d=vec3<f32>(dot(m[0].xyz,direction),dot(m[1].xyz,direction),dot(m[2].xyz,direction));
        if software_blas_any(software_ray(o,d),instance.info.x,instance.info.y,tmin,tmax) {return true;}
    }
    return false;
}
// Static rays: N0's software_occluded when nothing moves, so the cache traces N0's query.
fn cache_static(origin:vec3<f32>,direction:vec3<f32>,tmin:f32,tmax:f32)->bool {
    if cache_params[0].limits.w==0u {return software_occluded(origin,direction,tmin,tmax);}
    return cache_any(origin,direction,tmin,tmax,false);
}

// A cell's current state: rays in the disk, visible rays among them (static), and whether
// every ray in the disk is valid. `bits` and `info` are the stored words.
struct CacheState {inside:u32,visible:u32,complete:bool,window:vec2<i32>,disk:u32,blocked:u32};
fn cache_state(t:CacheLattice,bits:u32,info:u32)->CacheState {
    let window=cache_window(cache_centre(t));
    let disk=cache_disk(t,window);
    let old=cache_meta_window(info);
    let valid=cache_shift(bits>>16u,old,window);
    let blocked=cache_shift(bits&0xffffu,old,window);
    return CacheState(countOneBits(disk),countOneBits(disk&~blocked),(disk&~valid)==0u,window,disk,blocked);
}
// Visible rays after the movers: static-visible rays tested against each mover's BVH.
fn cache_composite(a:u32,b:u32,t:CacheLattice,s:CacheState)->u32 {
    if cache_params[0].limits.w==0u {return s.visible;}
    let r=cache_receiver(a,b);
    var visible=0u;var tested=0u;
    for(var bit=0u;bit<16u;bit+=1u) {
        if (s.disk&~s.blocked&(1u<<bit))==0u {continue;}
        let d=cache_direction(t,s.window+vec2<i32>(i32(bit&3u),i32(bit>>2u)));
        let o=cache_origin(r,d);
        tested+=1u;
        if !cache_any(o,d,r.epsilon,cache_tmax(o,d),true) {visible+=1u;}
    }
    atomicAdd(&cache_work[0].data[CACHE_MOVER_RAYS],tested);
    return visible;
}
fn cache_value(visible:u32,inside:u32)->u32 {
    if inside==0u {return 256u;}
    return (256u*visible+inside/2u)/inside;
}
// Queue a slot once per frame: its stamp moves to the current frame.
fn cache_enqueue(slot:u32,region:u32,fresh:bool) {
    let stamp=cache_params[0].frame.x;
    if !fresh {
        var info=atomicLoad(&cache_table[0].data[4u*slot+3u]);
        var won=false;
        for(var attempt=0u;attempt<4u;attempt+=1u) {
            if (info&0xffffu)==stamp {return;}
            let r=atomicCompareExchangeWeak(&cache_table[0].data[4u*slot+3u],info,(info&0xffff0000u)|stamp);
            if r.exchanged {won=true;break;}
            info=r.old_value;
        }
        if !won {return;}
    }
    let q=atomicAdd(&cache_work[0].data[CACHE_QUEUED+region],1u);
    if q<cache_params[0].limits.x {atomicStore(&cache_work[0].data[cache_queue(region)+q],slot);}
}
fn cache_defer(pixel:u32,slot:u32) {
    let q=atomicAdd(&cache_work[0].data[CACHE_PENDING],1u);
    if q<cache_params[0].limits.y {
        atomicStore(&cache_work[0].data[cache_pending_base()+2u*q],pixel);
        atomicStore(&cache_work[0].data[cache_pending_base()+2u*q+1u],slot);
    }
}
fn cache_movers(placement:u32)->bool {return software_instances[placement].info.w!=0u;}

// Prefetch: the sub-triangles sharing an edge with cell (i, j, upper) inside its triangle.
fn cache_prefetch(a:u32,cell:vec3<u32>,side:u32,level:u32) {
    let last=(1u<<level)-1u;
    for(var e=0u;e<3u;e+=1u) {
        var i=i32(cell.x);var j=i32(cell.y);var up=1u-cell.z;
        if cell.z==0u {
            if e==1u {i-=1;} if e==2u {j-=1;}
        } else {
            if e==1u {i+=1;} if e==2u {j+=1;}
        }
        if i<0 || j<0 {continue;}
        let n=u32(i)+u32(j);
        if n>last || (up!=0u && n==last) {continue;}
        let f=cache_find(a,cache_key_b(u32(i),u32(j),up,side),true);
        if f.y==CACHE_NEW {cache_enqueue(f.x,1u,true);}
        else if f.y==CACHE_FOUND {cache_enqueue(f.x,1u,false);}
    }
}

// Lookup and update: one invocation per pixel. Complete cells write the pixel's count; cold
// or incomplete cells are queued once and the pixel is deferred to cache_resolve.
@compute @workgroup_size(8,4) fn cache_lookup(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(primary_hit);
    if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);
    let hit=textureLoad(primary_hit,p,0);
    if hit.x==0u {textureStore(fresh_output,p,vec4<u32>(256u));return;}
    let placement=hit.x-1u;
    let pixel=id.x+id.y*size.x;
    if cache_movers(placement) {cache_defer(pixel,CACHE_NONE);return;}
    let v=software_positions(software_instances[placement].info.y,hit.y);
    let bary=bitcast<vec2<f32>>(hit.zw);
    let a=world_position(v[0],placement).xyz;let b=world_position(v[1],placement).xyz;let c=world_position(v[2],placement).xyz;
    let w=vec3<f32>(1.0-bary.x-bary.y,bary);
    let point=world_position(receiver_point(v[0],v[1],v[2],w),placement).xyz;
    let footprint=length(lighting.eye-point)*cache_params[0].view.x*cache_params[0].view.y;
    let level=cache_level(0.5*length(cross(b-a,c-a)),footprint);
    let geometric=primary_geometric(v[0],v[1],v[2],placement);
    let side=u32(dot(geometric,lighting.eye-point)<0.0);
    let cell=cache_cell(bary,level);
    let ka=cache_key_a(placement,hit.y,level);let kb=cache_key_b(cell.x,cell.y,cell.z,side);
    let f=cache_find(ka,kb,true);
    if f.y==CACHE_FOUND {
        let t=cache_lattice(ka,kb);
        let s=cache_state(t,atomicLoad(&cache_table[0].data[4u*f.x+2u]),atomicLoad(&cache_table[0].data[4u*f.x+3u]));
        if s.complete {
            textureStore(fresh_output,p,vec4<u32>(cache_value(cache_composite(ka,kb,t,s),s.inside)));
            atomicAdd(&cache_work[0].data[CACHE_WARM],1u);
            return;
        }
        atomicAdd(&cache_work[0].data[CACHE_INCOMPLETE],1u);
        cache_enqueue(f.x,0u,false);
        cache_defer(pixel,f.x);
        return;
    }
    if f.y==CACHE_NEW {
        cache_enqueue(f.x,0u,true);
        if (cache_params[0].frame.w&1u)!=0u && level>0u {cache_prefetch(ka,cell,side,level);}
        cache_defer(pixel,f.x);
        return;
    }
    atomicAdd(&cache_work[0].data[select(CACHE_BUSY_COUNT,CACHE_FULL_COUNT,f.y==CACHE_OVER)],1u);
    cache_defer(pixel,CACHE_NONE);
}

fn cache_groups(n:u32)->vec3<u32> {
    let g=max((n+63u)/64u,1u);let x=min(g,32768u);
    return vec3<u32>(x,(g+x-1u)/x,select(1u,0u,n==0u));
}
// One invocation: the fill takes visible requests first, then prefetch, up to the budget.
@compute @workgroup_size(1) fn cache_plan() {
    let cap=cache_params[0].limits.x;let budget=cache_params[0].frame.z;
    let primary=min(min(atomicLoad(&cache_work[0].data[CACHE_QUEUED]),cap),budget);
    let prefetch=min(min(atomicLoad(&cache_work[0].data[CACHE_QUEUED+1u]),cap),budget-primary);
    atomicStore(&cache_work[0].data[CACHE_FILL_PRIMARY],primary);
    atomicStore(&cache_work[0].data[CACHE_FILL_PREFETCH],prefetch);
    let fill=cache_groups(primary+prefetch);
    let pending=cache_groups(min(atomicLoad(&cache_work[0].data[CACHE_PENDING]),cache_params[0].limits.y));
    for(var k=0u;k<3u;k+=1u) {
        atomicStore(&cache_work[0].data[CACHE_FILL_ARGS+k],select(select(fill.z,fill.y,k==1u),fill.x,k==0u));
        atomicStore(&cache_work[0].data[CACHE_RESOLVE_ARGS+k],select(select(pending.z,pending.y,k==1u),pending.x,k==0u));
    }
}

// Fill: one invocation per planned cell. The window slides to the current sun; rays in the
// disk that are not valid are traced against static geometry.
@compute @workgroup_size(64) fn cache_fill(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let index=id.x+id.y*groups.x*64u;
    let primary=atomicLoad(&cache_work[0].data[CACHE_FILL_PRIMARY]);
    let total=primary+atomicLoad(&cache_work[0].data[CACHE_FILL_PREFETCH]);
    if index>=total {return;}
    let slot=atomicLoad(&cache_work[0].data[select(cache_queue(1u)+index-primary,cache_queue(0u)+index,index<primary)]);
    let w=4u*slot;
    let ka=atomicLoad(&cache_table[0].data[w]);let kb=atomicLoad(&cache_table[0].data[w+1u]);
    let bits=atomicLoad(&cache_table[0].data[w+2u]);let info=atomicLoad(&cache_table[0].data[w+3u]);
    let t=cache_lattice(ka,kb);
    let s=cache_state(t,bits,info);
    let old=cache_meta_window(info);
    var valid=cache_shift(bits>>16u,old,s.window);
    var blocked=s.blocked;
    let r=cache_receiver(ka,kb);
    var rays=0u;
    for(var bit=0u;bit<16u;bit+=1u) {
        let m=1u<<bit;
        if (s.disk&~valid&m)==0u {continue;}
        let d=cache_direction(t,s.window+vec2<i32>(i32(bit&3u),i32(bit>>2u)));
        let o=cache_origin(r,d);
        let tmax=cache_tmax(o,d);
        if tmax>r.epsilon && cache_static(o,d,r.epsilon,tmax) {blocked|=m;}
        valid|=m;rays+=1u;
    }
    atomicStore(&cache_table[0].data[w+2u],blocked|(valid<<16u));
    atomicStore(&cache_table[0].data[w+3u],cache_meta(info,s.window));
    atomicAdd(&cache_work[0].data[CACHE_FILLED],1u);
    atomicAdd(&cache_work[0].data[CACHE_RAYS],rays);
}

// Resolve: deferred pixels read their cell if the fill completed it, else take N0's
// Interactive estimate at the pixel's own receiver (the fallback).
@compute @workgroup_size(64) fn cache_resolve(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let index=id.x+id.y*groups.x*64u;
    if index>=min(atomicLoad(&cache_work[0].data[CACHE_PENDING]),cache_params[0].limits.y) {return;}
    let base=cache_pending_base()+2u*index;
    let pixel=atomicLoad(&cache_work[0].data[base]);let slot=atomicLoad(&cache_work[0].data[base+1u]);
    let size=textureDimensions(primary_hit);
    let p=vec2<i32>(vec2<u32>(pixel%size.x,pixel/size.x));
    if slot!=CACHE_NONE {
        let w=4u*slot;
        let ka=atomicLoad(&cache_table[0].data[w]);let kb=atomicLoad(&cache_table[0].data[w+1u]);
        let t=cache_lattice(ka,kb);
        let s=cache_state(t,atomicLoad(&cache_table[0].data[w+2u]),atomicLoad(&cache_table[0].data[w+3u]));
        if s.complete {
            textureStore(fresh_output,p,vec4<u32>(cache_value(cache_composite(ka,kb,t,s),s.inside)));
            return;
        }
    }
    atomicAdd(&cache_work[0].data[CACHE_FALLBACK],1u);
    let normal=textureLoad(fresh_normal,p,0);
    var value=256u;
    if normal.w!=0.0 {
        let world=textureLoad(fresh_world,p,0);
        let visible=fresh_estimate(Receiver(world.xyz,normal.xyz,p,0u,world.w,normal.w<0.0),vec2<u32>(p),size.x,0u);
        if visible!=FRESH_SKIP {value=(256u/FRESH_DIRECTIONS)*visible;}
    }
    textureStore(fresh_output,p,vec4<u32>(value));
}

// Validation replay: every valid ray of every claimed slot is traced again, in its stored
// window, with N0's query (static placements when movers exist), and compared.
@compute @workgroup_size(64) fn cache_validate(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let slot=id.x+id.y*groups.x*64u;
    if slot>cache_params[0].frame.y {return;}
    let w=4u*slot;
    let ka=atomicLoad(&cache_table[0].data[w]);
    if ka==0u || ka==CACHE_LOCK {return;}
    let kb=atomicLoad(&cache_table[0].data[w+1u]);
    let bits=atomicLoad(&cache_table[0].data[w+2u]);let info=atomicLoad(&cache_table[0].data[w+3u]);
    let t=cache_lattice(ka,kb);
    let r=cache_receiver(ka,kb);
    let window=cache_meta_window(info);
    var checked=0u;
    for(var bit=0u;bit<16u;bit+=1u) {
        let m=1u<<bit;
        if ((bits>>16u)&m)==0u {continue;}
        let d=cache_direction(t,window+vec2<i32>(i32(bit&3u),i32(bit>>2u)));
        let o=cache_origin(r,d);
        let tmax=cache_tmax(o,d);
        let blocked=tmax>r.epsilon && cache_static(o,d,r.epsilon,tmax);
        checked+=1u;
        if blocked!=((bits&m)!=0u) {
            let k=atomicAdd(&cache_work[0].data[CACHE_MISMATCH],1u);
            if k<4u {atomicStore(&cache_work[0].data[CACHE_DUMP+2u*k],slot);atomicStore(&cache_work[0].data[CACHE_DUMP+2u*k+1u],bit);}
        }
    }
    atomicAdd(&cache_work[0].data[CACHE_CHECKED],checked);
}
