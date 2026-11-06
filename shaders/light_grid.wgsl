// Light-space caster grid (LSG) for non-RT sun shadows: unit N4 of docs/non-rt-path-2.md.
// Test-only. src/light_grid.rs keeps the CPU reference (Grid::build, Grid::trace) and
// composes the query half after N0's traced module (shaders/software_bvh.wgsl), where it
// replaces the shadow rays' software any-hit call, or the build half as a module of its own.
//
// Heaps are binding arrays of 2^24-word chunks (64 MiB bindings). The front heap holds the
// fine cells' list ends (exclusive; a list begins at the previous cell's end), the max-top
// pyramid (f32 bits, finest level first) and the scan scratch. The entry heap holds
// (id, top bits, box) triples, each list sorted by (top descending, id ascending). Ids are
// placement << 28 | primitive. Tops and boxes are widened by the placement's margin; boxes
// are rounded outward to 8-bit steps of the cell enlarged by 1/16 on each side (quantize).

// u.w: cell size; v.w: 1 / cell; l.w: box quantum. dims: fine cells x, y, top level, query
// start level. sections: front words of the ends, tops, scan scratch and triangle cell
// ranges (two words each, written by the count pass and reused by the scatter). extra: entry
// capacity, unused, placements, triangles. levels[k/4][k%4]: level k's first word in the
// tops section. placements: first triangle, count, margin bits, mesh slot.
struct LightGrid {
    u:vec4<f32>,v:vec4<f32>,l:vec4<f32>,origin:vec4<f32>,
    dims:vec4<u32>,sections:vec4<u32>,extra:vec4<u32>,
    levels:array<vec4<u32>,8>,placements:array<vec4<u32>,16>,
};
const LIGHT_GRID_EMPTY:f32=-3.4028234663852886e38;
fn light_level(k:u32)->u32 {return light_grids[0].levels[k>>2u][k&3u];}
// Lower corner of fine cell `cell`'s quantization range, grid-relative.
fn light_box_base(cell:vec2<u32>)->vec2<f32> {return vec2<f32>(cell)*light_grids[0].u.w-vec2<f32>(light_grids[0].u.w*0.0625);}
// ---- query ----
struct LightWords {data:array<u32>};
// A one-element binding array: wgpu bind groups with binding arrays may not hold uniform
// buffers, and N0's layout already uses the stage's eight storage buffers.
@group(2) @binding(47) var<storage,read> light_grids:binding_array<LightGrid,1>;
@group(2) @binding(48) var<storage,read> light_front:binding_array<LightWords,LIGHT_GRID_FRONT_CHUNKS>;
@group(2) @binding(49) var<storage,read> light_entries:binding_array<LightWords,LIGHT_GRID_ENTRY_CHUNKS>;
fn light_front_word(a:u32)->u32 {return light_front[a>>24u].data[a&0xffffffu];}
fn light_entry_word(a:u32)->u32 {return light_entries[a>>24u].data[a&0xffffffu];}
// Steps after which a ray falls back to the BVH; light_grid.rs MAX_STEPS must match.
const LIGHT_GRID_MAX_STEPS:u32=4096u;
// Stats-run bounds only (they keep a single dispatch short): fewer steps, and a cap on
// entries read, past which a ray also falls back. Defaults leave the walk unchanged.
override LIGHT_GRID_STEP_LIMIT:u32=4096u;
override LIGHT_GRID_ENTRY_LIMIT:u32=0xffffffffu;

// The current placement's world-box verdict and object-space ray.
struct LightGridCache {placement:u32,boxed:bool,mesh:u32,ray:SoftwareRay};
// N0's any-hit test of one entry: its placement's world box, then the object-space ray and
// watertight test exactly as software_trace computes them.
fn light_grid_test(world:SoftwareRay,origin:vec3<f32>,direction:vec3<f32>,tmin:f32,tmax:f32,id:u32,cache:ptr<function,LightGridCache>)->bool {
    let placement=id>>28u;
    if placement!=(*cache).placement {
        (*cache).placement=placement;
        let instance=software_instances[placement];
        (*cache).boxed=software_box(world,instance.lo.xyz,instance.hi.xyz,tmin,tmax)<=tmax;
        let m=instance.rows;let p=vec4<f32>(origin,1.0);
        let o=vec3<f32>(dot(m[0],p),dot(m[1],p),dot(m[2],p));
        let d=vec3<f32>(dot(m[0].xyz,direction),dot(m[1].xyz,direction),dot(m[2].xyz,direction));
        (*cache).ray=software_ray(o,d);(*cache).mesh=instance.info.y;
    }
    if !(*cache).boxed {return false;}
    let v=software_positions((*cache).mesh,id&0x0fffffffu);
    return software_triangle((*cache).ray,v[0],v[1],v[2],tmin,tmax).x>=0.0;
}
fn light_fine(p:vec2<f32>,d:vec2<f32>,t:f32,n:vec2<i32>)->vec2<i32> {
    let x=floor(fma(d,vec2<f32>(t),p)*light_grids[0].v.w);
    return vec2<i32>(clamp(x,vec2<f32>(0.0),vec2<f32>(n-vec2<i32>(1))));
}
// Walk statistics of one ray, and rays that exhaust the step limit. light_grid_tests.rs
// swaps these bodies for atomic counters and a ray dump in its stats module; the probe's
// kernels keep the empty bodies.
fn light_grid_record(steps:u32,entries:u32,tests:u32,fallback:u32) {}
fn light_grid_dump(origin:vec3<f32>,direction:vec3<f32>,tmin:f32,tmax:f32) {}
fn light_grid_step(step:u32,level:u32,at:vec2<i32>,h:f32,rise:f32,t:f32) {}
// Never step back along the ray: positions only move with the ray's signs.
fn light_ahead(x:vec2<i32>,at:vec2<i32>,s:vec2<i32>)->vec2<i32> {return select(min(x,at),max(x,at),s>vec2<i32>(0));}
// Opaque any-hit with first-hit termination through the grid: Grid::trace in light_grid.rs.
// The ray rises along l; pyramid cells whose top lies below the ray where it enters them
// are skipped, and a fine cell's list is scanned until its tops fall below the ray.
fn light_grid_occluded(origin:vec3<f32>,direction:vec3<f32>,tmin:f32,tmax:f32)->bool {
    let world=software_ray(origin,direction);
    var cache=LightGridCache(0xffffffffu,false,0u,world);
    let p=vec2<f32>(dot(origin,light_grids[0].u.xyz),dot(origin,light_grids[0].v.xyz))-light_grids[0].origin.xy;
    let z0=dot(origin,light_grids[0].l.xyz);
    let d=vec2<f32>(dot(direction,light_grids[0].u.xyz),dot(direction,light_grids[0].v.xyz));
    let dz=dot(direction,light_grids[0].l.xyz);
    if !(dz>0.0) {light_grid_record(0u,0u,0u,1u);return software_occluded(origin,direction,tmin,tmax);}
    let tiny=select(vec2<f32>(-1e-30),vec2<f32>(1e-30),d>=vec2<f32>(0.0));
    let inv=vec2<f32>(1.0)/select(d,tiny,abs(d)<vec2<f32>(1e-30));
    let top_level=light_grids[0].dims.z;
    let size=vec2<f32>(f32(light_grids[0].dims.x),f32(light_grids[0].dims.y))*light_grids[0].u.w;
    let a0=-p*inv;let a1=(size-p)*inv;
    var t=max(max(min(a0.x,a1.x),min(a0.y,a1.y)),tmin);
    let root=bitcast<f32>(light_front_word(light_grids[0].sections.y+light_level(top_level)));
    let end=min(min(min(max(a0.x,a1.x),max(a0.y,a1.y)),tmax),(root-z0)/dz);
    if !(t<=end) {light_grid_record(0u,0u,0u,0u);return false;}
    let n=vec2<i32>(light_grids[0].dims.xy);
    let s=select(vec2<i32>(-1),vec2<i32>(1),d>=vec2<f32>(0.0));
    var at=light_fine(p,d,t,n);
    var level=min(light_grids[0].dims.w,top_level);
    var entries=0u;var tests=0u;
    for(var step=0u;step<min(LIGHT_GRID_MAX_STEPS,LIGHT_GRID_STEP_LIMIT);step+=1u) {
        let shift=vec2<u32>(level);
        let width=(light_grids[0].dims.x+(1u<<level)-1u)>>level;
        let cell=at>>shift;
        let h=bitcast<f32>(light_front_word(light_grids[0].sections.y+light_level(level)+u32(cell.x)+u32(cell.y)*width));
        let rise=fma(t,dz,z0);
        light_grid_step(step,level,at,h,rise,t);
        if h>=rise && level>0u {
            let first=cell<<shift;
            let last=min(((cell+vec2<i32>(1))<<shift)-vec2<i32>(1),n-vec2<i32>(1));
            at=clamp(light_ahead(light_fine(p,d,t,n),at,s),first,last);
            level-=1u;
            continue;
        }
        if h>=rise {
            let c=u32(at.x)+u32(at.y)*light_grids[0].dims.x;
            var e=0u;
            if c>0u {e=light_front_word(light_grids[0].sections.x+c-1u);}
            let stop=light_front_word(light_grids[0].sections.x+c);
            let base=light_box_base(vec2<u32>(at));
            let q=light_grids[0].l.w;
            for(;e<stop;e+=1u) {
                let top=bitcast<f32>(light_entry_word(3u*e+1u));
                entries+=1u;
                if entries>LIGHT_GRID_ENTRY_LIMIT {light_grid_record(step+1u,entries,tests,1u);return software_occluded(origin,direction,tmin,tmax);}
                if top<rise {break;}
                // The ray's part over the entry's box must pass below its top.
                let b=light_entry_word(3u*e+2u);
                let lo=fma(vec2<f32>(f32(b&255u),f32((b>>8u)&255u)),vec2<f32>(q),base);
                let hi=fma(vec2<f32>(f32((b>>16u)&255u),f32(b>>24u)),vec2<f32>(q),base);
                let s0=(lo-p)*inv;let s1=(hi-p)*inv;
                let near=max(max(min(s0.x,s1.x),min(s0.y,s1.y)),t);
                let far=min(min(max(s0.x,s1.x),max(s0.y,s1.y)),end);
                if near>far || top<fma(near,dz,z0) {continue;}
                tests+=1u;
                if light_grid_test(world,origin,direction,tmin,tmax,light_entry_word(3u*e),&cache) {light_grid_record(step+1u,entries,tests,0u);return true;}
            }
        }
        // Leave this level's cell through its nearer exit face.
        let side=light_grids[0].u.w*f32(1u<<level);
        let face=vec2<f32>(cell+select(vec2<i32>(0),vec2<i32>(1),s>vec2<i32>(0)))*side;
        let ex=(face-p)*inv;
        let leave=min(ex.x,ex.y);
        if leave>=end {light_grid_record(step+1u,entries,tests,0u);return false;}
        t=max(t,leave);
        // Step across the exit face; the other axis refines within the cell. Whole-vector
        // selects only: runtime-indexed component stores (at[axis]=...) left `at` unchanged
        // on NVIDIA 617.14 through naga's SPIR-V, so walks cycled to the step limit.
        let exit=vec2<bool>(ex.x<=ex.y,ex.y<ex.x);
        let counts=vec2<i32>(vec2<u32>(width,(light_grids[0].dims.y+(1u<<level)-1u)>>level));
        let next=cell+select(vec2<i32>(0),s,exit);
        if any(next<vec2<i32>(0)) || any(next>=counts) {light_grid_record(step+1u,entries,tests,0u);return false;}
        let first=next<<shift;
        let last=min(((next+vec2<i32>(1))<<shift)-vec2<i32>(1),n-vec2<i32>(1));
        at=select(clamp(light_ahead(light_fine(p,d,t,n),at,s),first,last),select(last,first,s>vec2<i32>(0)),exit);
        level=min(level+1u,top_level);
    }
    light_grid_dump(origin,direction,tmin,tmax);
    light_grid_record(LIGHT_GRID_MAX_STEPS,entries,tests,1u);
    return software_occluded(origin,direction,tmin,tmax);
}
// ---- build module ----
struct LightAtomics {data:array<atomic<u32>>};
struct LightVec4s {data:array<vec4<f32>>};
struct LightU32s {data:array<u32>};
// Entries are never updated atomically: each slot has one writer.
@group(0) @binding(0) var<storage,read> light_grids:binding_array<LightGrid,1>;
@group(0) @binding(1) var<storage,read_write> light_front:binding_array<LightAtomics,LIGHT_GRID_FRONT_CHUNKS>;
@group(0) @binding(2) var<storage,read_write> light_entries:binding_array<LightU32s,LIGHT_GRID_ENTRY_CHUNKS>;
// Placement models (object to world, as the renderer uploads them), and the meshes' own
// vertex (three vec4 per vertex, position first) and index buffers.
@group(0) @binding(3) var<storage,read> light_models:array<mat4x4<f32>>;
@group(0) @binding(4) var<storage,read> light_vertices:binding_array<LightVec4s,LIGHT_GRID_MESHES>;
@group(0) @binding(5) var<storage,read> light_indices:binding_array<LightU32s,LIGHT_GRID_MESHES>;
override LIGHT_LEVEL:u32=1u;
var<workgroup> light_scan:array<u32,256>;

fn light_load(a:u32)->u32 {return atomicLoad(&light_front[a>>24u].data[a&0xffffffu]);}
fn light_store(a:u32,x:u32) {atomicStore(&light_front[a>>24u].data[a&0xffffffu],x);}
fn light_add(a:u32,x:u32)->u32 {return atomicAdd(&light_front[a>>24u].data[a&0xffffffu],x);}
fn light_entry_load(a:u32)->u32 {return light_entries[a>>24u].data[a&0xffffffu];}
fn light_entry_store(a:u32,x:u32) {light_entries[a>>24u].data[a&0xffffffu]=x;}
fn light_index(id:vec3<u32>,groups:vec3<u32>)->u32 {return id.x+id.y*groups.x*256u;}
fn light_cells()->u32 {return light_grids[0].dims.x*light_grids[0].dims.y;}
fn light_flag()->u32 {return light_grids[0].sections.z+(light_cells()+1023u)/1024u+1u;}

// One triangle's widened light-space box (lo.xy, hi.xy), top, placement and primitive.
struct LightBox {lo:vec2<f32>,hi:vec2<f32>,top:f32,placement:u32,primitive:u32};
// Explicit fma only: the count and scatter passes must bin bit-identically.
fn light_dot(w:vec3<f32>,a:vec3<f32>)->f32 {return fma(w.x,a.x,fma(w.y,a.y,w.z*a.z));}
fn light_triangle(index:u32)->LightBox {
    var k=0u;
    for(var i=1u;i<light_grids[0].extra.z;i+=1u) {if index>=light_grids[0].placements[i].x {k=i;}}
    let placement=light_grids[0].placements[k];
    let t=index-placement.x;let mesh=placement.w;
    let m=light_models[k];
    var b=LightBox(vec2<f32>(3.4e38),vec2<f32>(-3.4e38),-3.4e38,k,t);
    for(var c=0u;c<3u;c+=1u) {
        let q=light_vertices[mesh].data[3u*light_indices[mesh].data[3u*t+c]].xyz;
        let w=fma(m[0].xyz,vec3<f32>(q.x),fma(m[1].xyz,vec3<f32>(q.y),fma(m[2].xyz,vec3<f32>(q.z),m[3].xyz)));
        let x=vec2<f32>(light_dot(w,light_grids[0].u.xyz),light_dot(w,light_grids[0].v.xyz));
        b.lo=min(b.lo,x);b.hi=max(b.hi,x);b.top=max(b.top,light_dot(w,light_grids[0].l.xyz));
    }
    let margin=bitcast<f32>(placement.z);
    b.lo-=vec2<f32>(margin);b.hi+=vec2<f32>(margin);b.top+=margin;
    return b;
}
// Fine-cell range (x0, y0, x1, y1), clamped to the grid.
fn light_range(b:LightBox)->vec4<u32> {
    let n=vec2<f32>(light_grids[0].dims.xy)-vec2<f32>(1.0);
    let lo=clamp(floor((b.lo-light_grids[0].origin.xy)*light_grids[0].v.w),vec2<f32>(0.0),n);
    let hi=clamp(floor((b.hi-light_grids[0].origin.xy)*light_grids[0].v.w),vec2<f32>(0.0),n);
    return vec4<u32>(vec2<u32>(lo),vec2<u32>(hi));
}
// light_grid.rs quantize in f32: the box rounded outward to 8-bit steps of the cell's range.
fn light_quantize(b:LightBox,cell:vec2<u32>)->u32 {
    let base=light_box_base(cell);
    let lo=clamp(floor((b.lo-light_grids[0].origin.xy-base)/light_grids[0].l.w),vec2<f32>(0.0),vec2<f32>(255.0));
    let hi=clamp(ceil((b.hi-light_grids[0].origin.xy-base)/light_grids[0].l.w),vec2<f32>(0.0),vec2<f32>(255.0));
    let q=vec4<u32>(vec2<u32>(lo),vec2<u32>(hi));
    return q.x|(q.y<<8u)|(q.z<<16u)|(q.w<<24u);
}
fn light_triangles()->u32 {
    let last=light_grids[0].placements[light_grids[0].extra.z-1u];
    return last.x+last.y;
}

// Zero the list counts and the overflow flag.
@compute @workgroup_size(256) fn light_grid_clear(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let c=light_index(id,groups);
    if c<light_cells() {light_store(light_grids[0].sections.x+c,0u);}
    if c==0u {light_store(light_flag(),0u);}
}
// One triangle per invocation: store its cell range and count its cells.
@compute @workgroup_size(256) fn light_grid_count(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let index=light_index(id,groups);
    if index>=light_triangles() {return;}
    let r=light_range(light_triangle(index));
    light_store(light_grids[0].sections.w+2u*index,r.x|(r.y<<16u));
    light_store(light_grids[0].sections.w+2u*index+1u,r.z|(r.w<<16u));
    for(var y=r.y;y<=r.w;y+=1u) {for(var x=r.x;x<=r.z;x+=1u) {light_add(light_grids[0].sections.x+x+y*light_grids[0].dims.x,1u);}}
}
// Exclusive scan of 1024 counts per workgroup, in place; block totals to the scratch.
@compute @workgroup_size(256) fn light_grid_scan(@builtin(workgroup_id) group:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let block=group.x+group.y*groups.x;
    let cells=light_cells();
    if block>=(cells+1023u)/1024u {return;}
    let base=block*1024u+lane*4u;
    var v:array<u32,4>;var sum=0u;
    for(var i=0u;i<4u;i+=1u) {
        v[i]=sum;
        if base+i<cells {sum+=light_load(light_grids[0].sections.x+base+i);}
    }
    light_scan[lane]=sum;
    workgroupBarrier();
    for(var o=1u;o<256u;o<<=1u) {
        var add=0u;
        if lane>=o {add=light_scan[lane-o];}
        workgroupBarrier();
        light_scan[lane]+=add;
        workgroupBarrier();
    }
    let prefix=light_scan[lane]-sum;
    for(var i=0u;i<4u;i+=1u) {if base+i<cells {light_store(light_grids[0].sections.x+base+i,prefix+v[i]);}}
    if lane==255u {light_store(light_grids[0].sections.z+block,light_scan[255]);}
}
// Exclusive scan of the block totals in one workgroup; the grand total follows them.
@compute @workgroup_size(256) fn light_grid_scan_sums(@builtin(local_invocation_index) lane:u32) {
    let blocks=(light_cells()+1023u)/1024u;
    let per=(blocks+255u)/256u;
    let first=lane*per;
    let sums=light_grids[0].sections.z;
    var sum=0u;
    for(var i=0u;i<per;i+=1u) {if first+i<blocks {sum+=light_load(sums+first+i);}}
    light_scan[lane]=sum;
    workgroupBarrier();
    for(var o=1u;o<256u;o<<=1u) {
        var add=0u;
        if lane>=o {add=light_scan[lane-o];}
        workgroupBarrier();
        light_scan[lane]+=add;
        workgroupBarrier();
    }
    var run=light_scan[lane]-sum;
    for(var i=0u;i<per;i+=1u) {
        if first+i<blocks {let x=light_load(sums+first+i);light_store(sums+first+i,run);run+=x;}
    }
    if lane==255u {light_store(sums+blocks,light_scan[255]);}
}
// Each count becomes its list's begin.
@compute @workgroup_size(256) fn light_grid_scan_add(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let c=light_index(id,groups);
    if c>=light_cells() {return;}
    let a=light_grids[0].sections.x+c;
    light_store(a,light_load(a)+light_load(light_grids[0].sections.z+c/1024u));
}
// The count pass again, appending entries: each list's begin advances to its end.
@compute @workgroup_size(256) fn light_grid_scatter(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let index=light_index(id,groups);
    if index>=light_triangles() {return;}
    let b=light_triangle(index);
    let lo=light_load(light_grids[0].sections.w+2u*index);let hi=light_load(light_grids[0].sections.w+2u*index+1u);
    let r=vec4<u32>(lo&0xffffu,lo>>16u,hi&0xffffu,hi>>16u);
    for(var y=r.y;y<=r.w;y+=1u) {
        for(var x=r.x;x<=r.z;x+=1u) {
            let slot=light_add(light_grids[0].sections.x+x+y*light_grids[0].dims.x,1u);
            if slot>=light_grids[0].extra.x {light_store(light_flag(),1u);continue;}
            light_entry_store(3u*slot,b.placement<<28u|b.primitive);
            light_entry_store(3u*slot+1u,bitcast<u32>(b.top));
            light_entry_store(3u*slot+2u,light_quantize(b,vec2<u32>(x,y)));
        }
    }
}
// (top descending, id ascending) before (top', id').
fn light_before(top:f32,id:u32,other_top:f32,other:u32)->bool {return top>other_top || (top==other_top && id<other);}
// Sort each list (insertion; lists are short) and publish its top as pyramid level 0.
@compute @workgroup_size(256) fn light_grid_sort(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let c=light_index(id,groups);
    if c>=light_cells() {return;}
    var begin=0u;
    if c>0u {begin=light_load(light_grids[0].sections.x+c-1u);}
    let end=min(light_load(light_grids[0].sections.x+c),light_grids[0].extra.x);
    for(var i=begin+1u;i<end;i+=1u) {
        let key=light_entry_load(3u*i);let top=bitcast<f32>(light_entry_load(3u*i+1u));let bbox=light_entry_load(3u*i+2u);
        var j=i;
        loop {
            if j==begin {break;}
            let other=light_entry_load(3u*j-3u);let other_top=bitcast<f32>(light_entry_load(3u*j-2u));
            if !light_before(top,key,other_top,other) {break;}
            light_entry_store(3u*j,other);light_entry_store(3u*j+1u,bitcast<u32>(other_top));
            light_entry_store(3u*j+2u,light_entry_load(3u*j-1u));
            j-=1u;
        }
        light_entry_store(3u*j,key);light_entry_store(3u*j+1u,bitcast<u32>(top));light_entry_store(3u*j+2u,bbox);
    }
    var top=LIGHT_GRID_EMPTY;
    if end>begin {top=bitcast<f32>(light_entry_load(3u*begin+1u));}
    light_store(light_grids[0].sections.y+c,bitcast<u32>(top));
}
// Pyramid level LIGHT_LEVEL from the level below: max of up to 2x2 children.
@compute @workgroup_size(256) fn light_grid_pyramid(@builtin(global_invocation_id) id:vec3<u32>,@builtin(num_workgroups) groups:vec3<u32>) {
    let c=light_index(id,groups);
    let k=LIGHT_LEVEL;
    let w=(light_grids[0].dims.x+(1u<<k)-1u)>>k;let h=(light_grids[0].dims.y+(1u<<k)-1u)>>k;
    if c>=w*h {return;}
    let cw=(light_grids[0].dims.x+(1u<<(k-1u))-1u)>>(k-1u);let ch=(light_grids[0].dims.y+(1u<<(k-1u))-1u)>>(k-1u);
    let x=c%w;let y=c/w;
    var m=LIGHT_GRID_EMPTY;
    for(var i=0u;i<4u;i+=1u) {
        let cx=2u*x+(i&1u);let cy=2u*y+(i>>1u);
        if cx<cw && cy<ch {m=max(m,bitcast<f32>(light_load(light_grids[0].sections.y+light_level(k-1u)+cx+cy*cw)));}
    }
    light_store(light_grids[0].sections.y+light_level(k)+c,bitcast<u32>(m));
}
