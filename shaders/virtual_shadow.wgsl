// N8: non-RT Interactive sun visibility (docs/non-rt-n8.md). No ray queries and no
// acceleration structure: each receiver's 8 Interactive directions get
//   - an exact watertight test against the receiver triangle and its vertex 1-ring,
//     over the whole ray (the existing edge adjacency, walked as vertex fans), and
//   - a short shadow-map march (SMRT-style) through a cached sun clipmap beyond
//     `near` metres, skipping map texels whose top triangle is in that 1-ring.
// Each level keeps two layers: the highest surface toward the sun (with its triangle id)
// and the highest surface facing away from it. A sample whose top surface is above the
// ray is confirmed exactly against that texel's triangle and its edge neighbours, or by
// the ray lying between the top and the back face of a thick occluder. Thin materials
// and open surfaces (no back face below the top) are sheets, decided exactly.
// The clipmap is page-driven like a virtual shadow map: receivers request 128-texel
// pages, only requested pages that are stale are rasterized (stencil-masked), and
// rasterized pages stay valid until the sun turns, the level window moves, or a
// moved object's light-space bounds touch them.
// virtual_shadow_gpu.rs prepends PRIMARY_BATCHES and owns every layout below.

const VSM_PAGE:u32=128u;
const VSM_NONE:u32=0xffffffffu;
const VSM_MAX_LEVELS:u32=8u;
const VSM_MAX_RECTS:u32=16u;
const VSM_MISSING:u32=0xffffffffu;
// Ring walk bound per fan direction; valence above this is truncated.
const VSM_FAN_STEPS:u32=24u;

// x0/y0: window minimum corner in eye-relative light-space metres; texel: metres.
struct VsmLevel {origin:vec4<f32>,info:vec4<u32>};
struct VsmFrame {
    u:vec4<f32>,v:vec4<f32>,l:vec4<f32>,
    // xyz: world eye; w: pixel footprint per metre of distance (2 tan(fov_y/2)/height).
    eye:vec4<f32>,
    // x: lowest caster height along l, y: 1/height range, z: height range, w: highest.
    depth:vec4<f32>,
    // x: near split r (m), y: finest texel s0 (m), z: disk slope, w: log2 texels per pixel.
    params:vec4<f32>,
    // x: levels, y: pages per window side, z: caster templates, w: invalidation rects.
    counts:vec4<u32>,
    // x: primitive bits of the id map, y: march samples per direction, z: bias in ulps of
    // height range, w: 1 to walk the receiver's vertex 1-ring, 0 for its own triangle only.
    misc:vec4<u32>,
    levels:array<VsmLevel,VSM_MAX_LEVELS>,
    // Eye-relative light-space xy rectangles (min.xy, max.xy) of moved casters.
    rects:array<vec4<f32>,VSM_MAX_RECTS>,
};
@group(0) @binding(0) var<uniform> vsm:VsmFrame;

fn vsm_light(world:vec3<f32>)->vec3<f32> {
    let rel=world-vsm.eye.xyz;
    return vec3<f32>(dot(rel,vsm.u.xyz),dot(rel,vsm.v.xyz),dot(world,vsm.l.xyz));
}
fn vsm_texel(level:u32)->f32 {return vsm.levels[level].origin.z;}
fn vsm_extent(level:u32)->f32 {return f32(vsm.counts.y*VSM_PAGE)*vsm_texel(level);}
// The march length a level can serve with its ±1-page neighbourhood marked.
fn vsm_reach(level:u32,height:f32)->f32 {
    let cap=f32(VSM_PAGE-4u)*vsm_texel(level)/max(vsm.params.z,1e-3);
    return max(0.0,min(cap,vsm.depth.w-height));
}
fn vsm_radius(level:u32,height:f32)->f32 {
    return vsm.params.z*vsm_reach(level,height)+3.0*vsm_texel(level);
}
// The finest level at about params.w texels per pixel whose window holds the receiver's
// whole march neighbourhood; coarser levels are the fallback near window edges.
fn vsm_select(world:vec3<f32>)->u32 {
    let p=vsm_light(world);
    let footprint=length(world-vsm.eye.xyz)*vsm.eye.w;
    let k=floor(log2(max(footprint,1e-9)/vsm.params.y)-vsm.params.w);
    let first=u32(clamp(k-f32(vsm.levels[0].info.y),0.0,f32(vsm.counts.x-1u)));
    for(var i=first;i<vsm.counts.x;i+=1u) {
        let r=vsm_radius(i,p.z);
        let lo=vsm.levels[i].origin.xy;let hi=lo+vec2<f32>(vsm_extent(i));
        if all(p.xy-vec2<f32>(r)>=lo) && all(p.xy+vec2<f32>(r)<hi) {return i;}
    }
    return VSM_NONE;
}

// ---- Page requests from the traced receivers ----
@group(0) @binding(1) var<storage,read_write> vsm_requests:array<u32>;
@group(0) @binding(7) var vsm_hit:texture_2d<u32>;
@group(0) @binding(8) var vsm_tiles:texture_2d<u32>;
@group(0) @binding(9) var vsm_world:texture_2d<f32>;
@group(0) @binding(10) var vsm_normal:texture_2d<f32>;
@group(0) @binding(14) var vsm_level_out:texture_storage_2d<r32uint,write>;
// primary_rays.wgsl PRIMARY_TILE.
const VSM_TRACE_TILE=vec2<u32>(8u,4u);

fn vsm_covered(id:vec2<u32>)->bool {
    return textureLoad(vsm_tiles,id/VSM_TRACE_TILE,0).x!=0u && textureLoad(vsm_hit,vec2<i32>(id),0).x!=0u;
}
@compute @workgroup_size(8,4) fn vsm_mark(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(vsm_world);
    if any(id.xy>=size) {return;}
    // Uncovered tiles never had hit records written; read their flag first.
    if textureLoad(vsm_tiles,id.xy/VSM_TRACE_TILE,0).x==0u {return;}
    var level=VSM_NONE;
    if textureLoad(vsm_hit,vec2<i32>(id.xy),0).x!=0u {
        let world=textureLoad(vsm_world,vec2<i32>(id.xy),0).xyz;
        level=vsm_select(world);
        if level!=VSM_NONE {
            let p=vsm_light(world);
            let r=vsm_radius(level,p.z);
            let page=f32(VSM_PAGE)*vsm_texel(level);
            let lo=vsm.levels[level].origin.xy;
            let pages=vsm.counts.y;
            let a=vec2<u32>(clamp(floor((p.xy-vec2<f32>(r)-lo)/page),vec2<f32>(0.0),vec2<f32>(f32(pages-1u))));
            let b=vec2<u32>(clamp(floor((p.xy+vec2<f32>(r)-lo)/page),vec2<f32>(0.0),vec2<f32>(f32(pages-1u))));
            for(var y=a.y;y<=b.y;y+=1u) {
                for(var x=a.x;x<=b.x;x+=1u) {
                    // Benign race: every writer stores the same value.
                    vsm_requests[(level*pages+y)*pages+x]=1u;
                }
            }
        }
    }
    // The lookup reads this level: one rounding of vsm_select for both passes.
    textureStore(vsm_level_out,vec2<i32>(id.xy),vec4<u32>(level));
}

// ---- Page cache: stale requested pages become this frame's raster list ----
struct VsmDirty {counts:array<atomic<u32>,VSM_MAX_LEVELS>,pages:array<u32>};
struct VsmDraw {vertex_count:u32,instance_count:u32,first_vertex:u32,first_instance:u32};
struct VsmIndexed {index_count:u32,instance_count:u32,first_index:u32,base_vertex:u32,first_instance:u32};
struct VsmTemplate {args:VsmIndexed,level:u32,pad0:u32,pad1:u32};
@group(0) @binding(2) var<storage,read_write> vsm_state:array<u32>;
@group(0) @binding(3) var<storage,read_write> vsm_dirty:VsmDirty;
@group(0) @binding(4) var<storage,read_write> vsm_quad_args:array<VsmDraw>;
@group(0) @binding(5) var<storage,read_write> vsm_caster_args:array<VsmIndexed>;
@group(0) @binding(6) var<storage,read> vsm_templates:array<VsmTemplate>;

fn vsm_moved(level:u32,page:vec2<u32>)->bool {
    let size=f32(VSM_PAGE)*vsm_texel(level);
    let lo=vsm.levels[level].origin.xy+vec2<f32>(page)*size;let hi=lo+vec2<f32>(size);
    for(var i=0u;i<vsm.counts.w;i+=1u) {
        let r=vsm.rects[i];
        if all(lo<=r.zw) && all(hi>=r.xy) {return true;}
    }
    return false;
}
@compute @workgroup_size(64) fn vsm_compact(@builtin(global_invocation_id) id:vec3<u32>) {
    let pages=vsm.counts.y;let per=pages*pages;
    let level=id.x/per;
    if level>=vsm.counts.x {return;}
    let local=id.x%per;
    let page=vec2<u32>(local%pages,local/pages);
    let epoch=vsm.levels[level].info.x;
    let moved=vsm_moved(level,page);
    if vsm_requests[id.x]!=0u {
        if vsm_state[id.x]!=epoch || moved {
            let n=atomicAdd(&vsm_dirty.counts[level],1u);
            vsm_dirty.pages[level*per+n]=local;
            vsm_state[id.x]=epoch;
        }
    } else if moved {
        // Not needed now, but its cached depths no longer describe the scene.
        vsm_state[id.x]=0u;
    }
}
@compute @workgroup_size(64) fn vsm_finalize(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x<vsm.counts.x {
        vsm_quad_args[id.x]=VsmDraw(6u,atomicLoad(&vsm_dirty.counts[id.x]),0u,0u);
    }
    if id.x<vsm.counts.z {
        let t=vsm_templates[id.x];
        var args=t.args;
        // Levels without stale requested pages issue zero-instance draws: no vertex work.
        if atomicLoad(&vsm_dirty.counts[t.level])==0u {args.instance_count=0u;}
        vsm_caster_args[id.x]=args;
    }
}

// ---- Page raster: one pass per level, stencil-masked to the stale pages ----
struct VsmPages {counts:array<u32,VSM_MAX_LEVELS>,pages:array<u32>};
@group(0) @binding(11) var<storage,read> vsm_raster_pages:VsmPages;
@group(0) @binding(12) var<storage,read> vsm_models:array<mat4x4<f32>>;
struct VsmPass {level:u32,pad0:u32,pad1:u32,pad2:u32};
@group(1) @binding(30) var<uniform> vsm_pass:VsmPass;

fn vsm_clip(level:u32,p:vec3<f32>)->vec4<f32> {
    let origin=vsm.levels[level].origin.xy;let extent=vsm_extent(level);
    let xy=(p.xy-origin)/extent;
    return vec4<f32>(xy.x*2.0-1.0,1.0-xy.y*2.0,(p.z-vsm.depth.x)*vsm.depth.y,1.0);
}
@vertex fn vs_vsm_clear(@builtin(vertex_index) corner:u32,@builtin(instance_index) n:u32)->@builtin(position) vec4<f32> {
    let level=vsm_pass.level;let pages=vsm.counts.y;
    let local=vsm_raster_pages.pages[level*pages*pages+n];
    let page=vec2<u32>(local%pages,local/pages);
    var corners=array<vec2<u32>,6>(vec2<u32>(0u,0u),vec2<u32>(1u,0u),vec2<u32>(0u,1u),vec2<u32>(0u,1u),vec2<u32>(1u,0u),vec2<u32>(1u,1u));
    // Page corners are exact binary fractions of the window.
    let xy=vec2<f32>(page+corners[corner])/f32(pages);
    return vec4<f32>(xy.x*2.0-1.0,1.0-xy.y*2.0,0.0,1.0);
}
@fragment fn fs_vsm_clear()->@location(0) u32 {return 0u;}

struct VsmCaster {@builtin(position) position:vec4<f32>,@location(0) @interpolate(flat) placement:u32,@location(1) light:vec3<f32>};
@vertex fn vs_vsm_caster(@location(0) position:vec3<f32>,@builtin(instance_index) placement:u32)->VsmCaster {
    let world=(vsm_models[placement]*vec4<f32>(position,1.0)).xyz;
    let p=vsm_light(world);
    return VsmCaster(vsm_clip(vsm_pass.level,p),placement,p);
}
// The back layer keeps only triangles whose source winding faces away from the sun.
struct VsmBackTriangle {@location(1) @interpolate(per_vertex) light:array<vec3<f32>,3>};
@fragment fn fs_vsm_back(triangle:VsmBackTriangle) {
    let a=triangle.light[0];let b=triangle.light[1];let c=triangle.light[2];
    if cross(b-a,c-a).z>=0.0 {discard;}
}
// Placement + 1 and primitive, so the lookup can recognize the receiver's own 1-ring.
@fragment fn fs_vsm_caster(input:VsmCaster,@builtin(primitive_index) primitive:u32)->@location(0) u32 {
    return ((input.placement+1u)<<vsm.misc.x)|primitive;
}

// ---- Lookup: exact 1-ring, then the map march beyond `near` ----
@group(0) @binding(15) var vsm_output:texture_storage_2d<r32uint,write>;
@group(0) @binding(16) var vsm_depth:texture_depth_2d_array;
@group(0) @binding(17) var vsm_ids:texture_2d_array<u32>;
@group(0) @binding(18) var vsm_level_in:texture_2d<u32>;
@group(0) @binding(19) var vsm_back:texture_depth_2d_array;

struct NormalTransform {x:vec4<f32>,y:vec4<f32>,z:vec4<f32>,info:vec4<f32>};
struct VsmVec4s {data:array<vec4<f32>>};
struct VsmU32s {data:array<u32>};
@group(1) @binding(0) var<storage,read> models:array<mat4x4<f32>>;
@group(1) @binding(2) var<storage,read> primary_vertices:binding_array<VsmVec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> primary_indices:binding_array<VsmU32s,PRIMARY_BATCHES>;
@group(1) @binding(5) var<storage,read> primary_placements:array<u32>;

struct LightingUniform {eye:vec3<f32>,count:u32,environment:vec3<f32>,exposure:f32,map:vec4<f32>};
struct Light {position_range:vec4<f32>,travel_outer:vec4<f32>,rgb_inner:vec4<f32>,kind:vec4<u32>};
struct ShadowView {world_to_clip:mat4x4<f32>,coverage_bias:vec4<f32>,camera_depth:vec4<f32>,texel:vec4<f32>};
@group(3) @binding(0) var<uniform> lighting:LightingUniform;
@group(3) @binding(1) var<storage,read> lights:array<Light>;
@group(3) @binding(5) var<storage,read> shadow_views:array<ShadowView>;

fn vsm_triangle(batch:u32,primitive:u32)->array<vec3<f32>,3> {
    let base=primitive*3u;
    return array<vec3<f32>,3>(primary_vertices[batch].data[primary_indices[batch].data[base]*3u].xyz,
        primary_vertices[batch].data[primary_indices[batch].data[base+1u]*3u].xyz,
        primary_vertices[batch].data[primary_indices[batch].data[base+2u]*3u].xyz);
}
// The adjacency suffix of the index buffer (receiver_adjacency.rs): 6T+1 words.
fn vsm_neighbour(batch:u32,primitive:u32,edge:u32)->u32 {
    let words=arrayLength(&primary_indices[batch].data);
    if words%3u!=1u {return VSM_MISSING;}
    return primary_indices[batch].data[(words-1u)/2u+primitive*3u+edge];
}
fn vsm_unit(n:vec3<f32>)->vec3<f32> {
    let scale=max(max(abs(n.x),abs(n.y)),abs(n.z));
    let scaled=n/max(scale,1e-30);
    return scaled*inverseSqrt(max(dot(scaled,scaled),1e-30));
}
// fresh_shadows.wgsl's per-pixel R2 sequence, so N8 traces Interactive's directions.
fn vsm_hash(input:u32)->u32 {
    var x=input;x=(x^(x>>16u))*0x7feb352du;x=(x^(x>>15u))*0x846ca68bu;return x^(x>>16u);
}
fn vsm_direction(index:u32,shift:vec2<f32>,l:vec3<f32>,u:vec3<f32>,v:vec3<f32>,slope:f32)->vec3<f32> {
    let r=fract(vec2<f32>(f32(index))*vec2<f32>(0.7548776662466927,0.5698402909980532)+shift);
    let a=6.283185307179586*r.y;
    return normalize(l+slope*sqrt(r.x)*(cos(a)*u+sin(a)*v));
}

// The watertight ray/triangle test of the N0 software backend (Woop, Benthin and Wald
// 2013), unchanged: shared edges give opposite-signed or zero edge functions.
struct VsmRay {origin:vec3<f32>,direction:vec3<f32>,shear:vec3<f32>,axes:vec3<u32>};
fn vsm_ray(origin:vec3<f32>,direction:vec3<f32>)->VsmRay {
    let a=abs(direction);
    var z=2u;
    if a.x>=a.y && a.x>=a.z {z=0u;} else if a.y>=a.z {z=1u;}
    var x=(z+1u)%3u;var y=(z+2u)%3u;
    if direction[z]<0.0 {let s=x;x=y;y=s;}
    let dz=direction[z];
    return VsmRay(origin,direction,vec3<f32>(direction[x]/dz,direction[y]/dz,1.0/dz),vec3<u32>(x,y,z));
}
fn vsm_difference(a:f32,b:f32,c:f32,d:f32)->f32 {
    let w=c*d;
    return fma(a,b,-w)+fma(-c,d,w);
}
fn vsm_hits(ray:VsmRay,p0:vec3<f32>,p1:vec3<f32>,p2:vec3<f32>,tmin:f32)->bool {
    let k=ray.axes;let s=ray.shear;
    let a=p0-ray.origin;let b=p1-ray.origin;let c=p2-ray.origin;
    let ax=fma(-s.x,a[k.z],a[k.x]);let ay=fma(-s.y,a[k.z],a[k.y]);
    let bx=fma(-s.x,b[k.z],b[k.x]);let by=fma(-s.y,b[k.z],b[k.y]);
    let cx=fma(-s.x,c[k.z],c[k.x]);let cy=fma(-s.y,c[k.z],c[k.y]);
    let u=vsm_difference(cx,by,cy,bx);
    let v=vsm_difference(ax,cy,ay,cx);
    let w=vsm_difference(bx,ay,by,ax);
    if (u<0.0 || v<0.0 || w<0.0) && (u>0.0 || v>0.0 || w>0.0) {return false;}
    let det=u+v+w;
    if det==0.0 {return false;}
    let t=fma(u,s.z*a[k.z],fma(v,s.z*b[k.z],w*(s.z*c[k.z])))/det;
    return t>=tmin;
}

struct VsmReceiver {world:vec3<f32>,geometric:vec3<f32>,epsilon:f32,thin:bool,slot:u32,batch:u32,primitive:u32};
// shadow_origin (scene.wgsl): opaque receivers stay on the viewed side.
fn vsm_origin(r:VsmReceiver,toward_light:vec3<f32>)->vec3<f32> {
    let toward=select(lighting.eye-r.world,toward_light,r.thin);
    let normal=select(-r.geometric,r.geometric,dot(r.geometric,toward)>=0.0);
    return fma(normal,vec3<f32>(r.epsilon),r.world);
}
fn vsm_to_world(v:vec3<f32>,slot:u32)->vec3<f32> {return (models[slot]*vec4<f32>(v,1.0)).xyz;}
fn vsm_same(a:vec3<f32>,b:vec3<f32>)->bool {return all(a==b);}

// Per-pixel lookup state, set once by vsm_lookup: the eight directions, the receiver
// triangle's object-space positions and the opaque receivers' shared origin. Private
// storage avoids copying arrays into every call.
var<private> vsm_dirs:array<vec3<f32>,8>;
var<private> vsm_own:array<vec3<f32>,3>;
var<private> vsm_shared_origin:vec3<f32>;
// The current direction's origin for vsm_triangle_hits (thin receivers move it per ray).
var<private> vsm_thin_origin:vec3<f32>;
fn vsm_ray_origin_private(j:u32)->vec3<f32> {return vsm_thin_origin;}
// One axis permutation for all eight directions: the sun's dominant axis (directions are
// within 0.1 rad of it), with x/y swapped when it points down so edge signs keep their
// meaning. Each direction keeps its own shear. Watertightness needs only consistency
// between triangles for one ray, which a fixed permutation keeps.
var<private> vsm_axes:vec3<u32>;
var<private> vsm_shear:array<vec3<f32>,8>;
// The central direction's shear, and the sheared-plane radius per unit height of the cone
// holding all eight directions.
var<private> vsm_centre:vec3<f32>;
var<private> vsm_cone:f32;
fn vsm_permute(v:vec3<f32>)->vec3<f32> {return vec3<f32>(v[vsm_axes.x],v[vsm_axes.y],v[vsm_axes.z]);}
fn vsm_prepare_rays(l:vec3<f32>) {
    let a=abs(l);
    var z=2u;
    if a.x>=a.y && a.x>=a.z {z=0u;} else if a.y>=a.z {z=1u;}
    var x=(z+1u)%3u;var y=(z+2u)%3u;
    if l[z]<0.0 {let t=x;x=y;y=t;}
    vsm_axes=vec3<u32>(x,y,z);
    let c=vsm_permute(l);
    vsm_centre=vec3<f32>(c.x/c.z,c.y/c.z,1.0/c.z);
    var cone=0.0;
    for(var j=0u;j<8u;j+=1u) {
        let d=vsm_permute(vsm_dirs[j]);
        let shear=vec3<f32>(d.x/d.z,d.y/d.z,1.0/d.z);
        cone=max(cone,length(shear.xy-vsm_centre.xy));
        // Constant-index stores only: unrolled by hand below.
        if j==0u {vsm_shear[0]=shear;} else if j==1u {vsm_shear[1]=shear;} else if j==2u {vsm_shear[2]=shear;}
        else if j==3u {vsm_shear[3]=shear;} else if j==4u {vsm_shear[4]=shear;} else if j==5u {vsm_shear[5]=shear;}
        else if j==6u {vsm_shear[6]=shear;} else {vsm_shear[7]=shear;}
    }
    vsm_cone=cone/abs(vsm_centre.z);
}
// The watertight test on a triangle already permuted and made relative to the origin.
fn vsm_hits_sheared(s:vec3<f32>,a:vec3<f32>,b:vec3<f32>,c:vec3<f32>,tmin:f32)->bool {
    let ax=fma(-s.x,a.z,a.x);let ay=fma(-s.y,a.z,a.y);
    let bx=fma(-s.x,b.z,b.x);let by=fma(-s.y,b.z,b.y);
    let cx=fma(-s.x,c.z,c.x);let cy=fma(-s.y,c.z,c.y);
    let u=vsm_difference(cx,by,cy,bx);
    let v=vsm_difference(ax,cy,ay,cx);
    let w=vsm_difference(bx,ay,by,ax);
    if (u<0.0 || v<0.0 || w<0.0) && (u>0.0 || v>0.0 || w>0.0) {return false;}
    let det=u+v+w;
    if det==0.0 {return false;}
    let t=fma(u,s.z*a.z,fma(v,s.z*b.z,w*(s.z*c.z)))/det;
    return t>=tmin;
}
fn vsm_ray_origin(r:VsmReceiver,j:u32)->vec3<f32> {
    if r.thin {return vsm_origin(r,vsm_dirs[j]);}
    return vsm_shared_origin;
}
// Bit j set: direction j is blocked by this world-space triangle.
// Also grows the ring's squared reach: the sheared projection along the sun overestimates
// light-space distance, so it is a safe bound for the map's ring filter.
fn vsm_test(r:VsmReceiver,a:vec3<f32>,b:vec3<f32>,c:vec3<f32>,ring:VsmRing)->VsmRing {
    var result=ring;
    let o=vsm_shared_origin;
    let pa=vsm_permute(a-o);let pb=vsm_permute(b-o);let pc=vsm_permute(c-o);
    let s0=vsm_centre;
    let qa=vec2<f32>(fma(-s0.x,pa.z,pa.x),fma(-s0.y,pa.z,pa.y));
    let qb=vec2<f32>(fma(-s0.x,pb.z,pb.x),fma(-s0.y,pb.z,pb.y));
    let qc=vec2<f32>(fma(-s0.x,pc.z,pc.x),fma(-s0.y,pc.z,pc.y));
    result.radius=max(result.radius,max(dot(qa,qa),max(dot(qb,qb),dot(qc,qc))));
    // Cone cull: the rays rise to height h = t (sheared z), within vsm_cone*h of the central
    // ray. Thin origins differ by up to 2 epsilon; the pad also covers rounding.
    let h=max(pa.z*s0.z,max(pb.z*s0.z,pc.z*s0.z));
    let pad=4.0*r.epsilon+1e-5*max(max(length(pa),length(pb)),length(pc));
    if h<r.epsilon-pad {return result;}
    let gap=max(max(min(qa,min(qb,qc)),-max(qa,max(qb,qc))),vec2<f32>(0.0));
    let reach=vsm_cone*max(h,0.0)+pad;
    if dot(gap,gap)>reach*reach {return result;}
    for(var j=0u;j<8u;j+=1u) {
        if (result.blocked&(1u<<j))!=0u {continue;}
        var hit=false;
        if r.thin {
            let oj=vsm_ray_origin(r,j);
            hit=vsm_hits_sheared(vsm_shear[j],vsm_permute(a-oj),vsm_permute(b-oj),vsm_permute(c-oj),r.epsilon);
        } else {
            hit=vsm_hits_sheared(vsm_shear[j],pa,pb,pc,r.epsilon);
        }
        if hit {result.blocked|=1u<<j;}
    }
    return result;
}
// The ring's blocked directions and its light-space reach around the receiver.
struct VsmRing {blocked:u32,radius:f32};
// Squared light-space reach; vsm_ring returns its square root.
fn vsm_reach_of(radius:f32,centre:vec2<f32>,a:vec3<f32>,b:vec3<f32>,c:vec3<f32>)->f32 {
    let pa=vsm_light(a).xy-centre;let pb=vsm_light(b).xy-centre;let pc=vsm_light(c).xy-centre;
    return max(radius,max(dot(pa,pa),max(dot(pb,pb),dot(pc,pc))));
}
fn vsm_indices(batch:u32,primitive:u32)->vec3<u32> {
    let base=primitive*3u;
    return vec3<u32>(primary_indices[batch].data[base],primary_indices[batch].data[base+1u],primary_indices[batch].data[base+2u]);
}
fn vsm_position(batch:u32,index:u32)->vec3<f32> {return primary_vertices[batch].data[index*3u].xyz;}
// One ring triangle from object-space positions; vsm_test culls it against the ray cone.
fn vsm_ring_triangle(r:VsmReceiver,v:array<vec3<f32>,3>,ring:VsmRing)->VsmRing {
    return vsm_test(r,vsm_to_world(v[0],r.slot),vsm_to_world(v[1],r.slot),vsm_to_world(v[2],r.slot),ring);
}
fn vsm_corner(w:array<vec3<f32>,3>,p:vec3<f32>)->u32 {
    return select(select(select(3u,2u,vsm_same(w[2],p)),1u,vsm_same(w[1],p)),0u,vsm_same(w[0],p));
}
// The receiver triangle, then every triangle sharing one of its vertex positions, reached
// by walking each vertex fan across the exact-position edge adjacency in both
// directions. Positions, not indices, identify vertices, so fans cross duplicated seam
// vertices as the adjacency does and as vsm_in_ring assumes. A triangle across edge
// (c, c+1) is tested only in the fan of c+1.
fn vsm_ring(r:VsmReceiver)->VsmRing {
    var ring=vsm_ring_triangle(r,vsm_own,VsmRing(0u,0.0));
    for(var c=0u;c<3u*vsm.misc.w;c+=1u) {
        if ring.blocked==0xffu {break;}
        let p=vsm_own[c];
        let skip=vsm_own[(c+1u)%3u];
        var closed=false;
        for(var side=0u;side<2u && !closed;side+=1u) {
            var t=r.primitive;
            // Leaving across edge (c+1) shares {c, c+2}; across edge (c+2) shares {c, c+1}.
            var edge=(c+1u+side)%3u;
            var q=vsm_own[(c+2u-side)%3u];
            for(var step=0u;step<VSM_FAN_STEPS;step+=1u) {
                let n=vsm_neighbour(r.batch,t,edge);
                if n==VSM_MISSING {break;}
                if n==r.primitive {closed=true;break;}
                var w=vsm_triangle(r.batch,n);
                let cp=vsm_corner(w,p);let cq=vsm_corner(w,q);
                if cp==3u || cq==3u || cp==cq {break;}
                if vsm_corner(w,skip)==3u {ring=vsm_ring_triangle(r,w,ring);}
                edge=cq;q=w[3u-cp-cq];t=n;
            }
        }
    }
    ring.radius=sqrt(ring.radius);
    return ring;
}
// Whether a map texel's top triangle shares a vertex position with the receiver's.
fn vsm_in_ring(r:VsmReceiver,id:u32)->bool {
    let placement=(id>>vsm.misc.x)-1u;
    if placement!=r.slot {return false;}
    let primitive=id&((1u<<vsm.misc.x)-1u);
    if primitive==r.primitive {return true;}
    var w=vsm_triangle(r.batch,primitive);
    for(var i=0u;i<3u;i+=1u) {
        if vsm_same(w[i],vsm_own[0]) || vsm_same(w[i],vsm_own[1]) || vsm_same(w[i],vsm_own[2]) {return true;}
    }
    return false;
}
// Whether direction j's ray beyond `near` crosses the map texel's top triangle, exactly.
// A steep or open surface crosses the ray below its texel's highest point, between march
// samples, where the thickness test cannot see it.
// Its edge neighbours are tested too: the crossing can lie just across an edge (a quad's
// diagonal) from the triangle that owns the sampled texel.
fn vsm_triangle_hits(batch:u32,primitive:u32,placement:u32,j:u32)->bool {
    let v=vsm_triangle(batch,primitive);
    let m=models[placement];
    let o=vsm_ray_origin_private(j);
    let a=vsm_permute((m*vec4<f32>(v[0],1.0)).xyz-o);
    let b=vsm_permute((m*vec4<f32>(v[1],1.0)).xyz-o);
    let c=vsm_permute((m*vec4<f32>(v[2],1.0)).xyz-o);
    return vsm_hits_sheared(vsm_shear[j],a,b,c,vsm.params.x);
}
fn vsm_top_hits(r:VsmReceiver,id:u32,placement:u32,j:u32)->bool {
    vsm_thin_origin=select(vsm_shared_origin,vsm_ray_origin(r,j),r.thin);
    let primitive=id&((1u<<vsm.misc.x)-1u);
    let batch=primary_placements[placement]&0x1fffffffu;
    if vsm_triangle_hits(batch,primitive,placement,j) {return true;}
    for(var e=0u;e<3u;e+=1u) {
        let n=vsm_neighbour(batch,primitive,e);
        if n!=VSM_MISSING && vsm_triangle_hits(batch,n,placement,j) {return true;}
    }
    return false;
}
// SMRT-style march of one direction beyond `near`: geometric steps to the highest caster
// or the level's page reach. Step i covers the ray heights since the previous step; its
// texel is a candidate when the top surface is above that range (and above the receiver's
// own plane at the texel centre). Most lit samples cost one depth load.
fn vsm_far(r:VsmReceiver,ring:f32,level:u32,j:u32)->bool {
    let d=vsm_dirs[j];
    let p=vsm_light(vsm_ray_origin(r,j));
    let step=vec3<f32>(dot(d,vsm.u.xyz),dot(d,vsm.v.xyz),dot(d,vsm.l.xyz));
    let near=vsm.params.x;
    if step.z<=0.0 {return false;}
    let reach=vsm_reach(level,p.z)/step.z;
    if reach<=near {return false;}
    let texel=vsm_texel(level);
    let lo=vsm.levels[level].origin.xy;
    let size=i32(vsm.counts.y*VSM_PAGE);
    // The origin-side receiver plane in light space, for texel-centre extrapolation.
    let toward=select(lighting.eye-r.world,d,r.thin);
    let n=select(-r.geometric,r.geometric,dot(r.geometric,toward)>=0.0);
    let nl=vec3<f32>(dot(n,vsm.u.xyz),dot(n,vsm.v.xyz),dot(n,vsm.l.xyz));
    let facing=nl.z>0.0;
    let gradient=-nl.xy/max(nl.z,0.05);
    let bias=f32(vsm.misc.z)*vsm.depth.z*5.960464477539063e-8;
    let samples=max(vsm.misc.y,1u);
    let ratio=pow(reach/near,1.0/f32(samples));
    let bits=vsm.misc.x;
    var tested=0u;
    var t=near;
    var below=fma(step.z,near,p.z);
    for(var i=0u;i<samples;i+=1u) {
        t*=ratio;
        let q=fma(step,vec3<f32>(t),p);
        let cell=vec2<i32>(floor((q.xy-lo)/texel));
        var lower=below;
        below=q.z;
        if any(cell<vec2<i32>(0)) || any(cell>=vec2<i32>(size)) {continue;}
        if facing {
            let centre=lo+(vec2<f32>(cell)+vec2<f32>(0.5))*texel;
            lower=max(lower,p.z+dot(gradient,centre-p.xy));
        }
        let top=fma(textureLoad(vsm_depth,cell,level,0),vsm.depth.z,vsm.depth.x);
        if top<=lower+bias {continue;}
        let id=textureLoad(vsm_ids,cell,level,0).x;
        if id==0u {continue;}
        let placement=(id>>bits)-1u;
        // Ring members were tested exactly over the whole ray; without the ring walk only the
        // receiver's own triangle was, and its neighbours are decided here like any caster.
        if placement==r.slot && (id&((1u<<bits)-1u))==r.primitive {continue;}
        if vsm.misc.w!=0u && placement==r.slot && distance(q.xy,p.xy)<=ring+2.0*texel && vsm_in_ring(r,id) {continue;}
        // The texel's top triangle (and its edge neighbours) is the candidate: a ray below
        // a surface leaves through it, so an exact hit decides without texel quantization.
        if id!=tested {
            tested=id;
            if vsm_top_hits(r,id,placement,j) {return true;}
        }
        // Inside a thick occluder at this sample: between its top and the highest surface
        // facing away from the sun below it. Thin materials and open surfaces (no back face
        // below the top; a closed mesh always has one) are sheets, decided above.
        if (primary_placements[placement]&0x20000000u)==0u {
            let back=textureLoad(vsm_back,cell,level,0);
            let bottom=fma(back,vsm.depth.z,vsm.depth.x);
            if back>0.0 && bottom<top-4.0*bias && q.z>=bottom-bias {return true;}
        }
    }
    return false;
}

@compute @workgroup_size(8,4) fn vsm_lookup(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(vsm_world);
    if any(id.xy>=size) {return;}
    if textureLoad(vsm_tiles,id.xy/VSM_TRACE_TILE,0).x==0u {return;}
    let p=vec2<i32>(id.xy);
    let record=textureLoad(vsm_hit,p,0);
    let normal=textureLoad(vsm_normal,p,0);
    var visible=256u;
    let light=lights[0];
    if record.x!=0u && normal.w!=0.0 && light.kind.z!=0u {
        let world=textureLoad(vsm_world,p,0);
        // fresh_estimate's cascade coverage and light basis, so directions match Interactive.
        let depth=dot(shadow_views[light.kind.y].camera_depth,vec4<f32>(world.xyz,1.0));
        var layer=VSM_NONE;
        for(var i=0u;i<light.kind.z;i+=1u) {
            let coverage=shadow_views[light.kind.y+i].coverage_bias;
            if depth>=coverage.x && depth<=coverage.y {layer=light.kind.y+i;break;}
        }
        if layer!=VSM_NONE {
            let view=shadow_views[layer];
            let slot=record.x-1u;
            let r=VsmReceiver(world.xyz,normal.xyz,world.w,normal.w<0.0,slot,primary_placements[slot]&0x1fffffffu,record.y);
            let l=-light.travel_outer.xyz;
            let rows=transpose(view.world_to_clip);let u=normalize(rows[0].xyz);let v=normalize(rows[1].xyz);
            let seed=vsm_hash(id.x+id.y*size.x);
            let shift=vec2<f32>(f32(seed>>8u),f32(vsm_hash(seed^0xa511e9b3u)>>8u))*(1.0/16777216.0);
            let slope=view.texel.z;
            // Constant-index stores only (docs: runtime-indexed stores misbehaved on NVIDIA).
            vsm_dirs[0]=vsm_direction(0u,shift,l,u,v,slope);vsm_dirs[1]=vsm_direction(1u,shift,l,u,v,slope);
            vsm_dirs[2]=vsm_direction(2u,shift,l,u,v,slope);vsm_dirs[3]=vsm_direction(3u,shift,l,u,v,slope);
            vsm_dirs[4]=vsm_direction(4u,shift,l,u,v,slope);vsm_dirs[5]=vsm_direction(5u,shift,l,u,v,slope);
            vsm_dirs[6]=vsm_direction(6u,shift,l,u,v,slope);vsm_dirs[7]=vsm_direction(7u,shift,l,u,v,slope);
            vsm_own=vsm_triangle(r.batch,r.primitive);
            vsm_shared_origin=vsm_origin(r,l);
            vsm_prepare_rays(l);
            let ring=vsm_ring(r);
            var blocked=ring.blocked;
            let level=textureLoad(vsm_level_in,p,0).x;
            if level!=VSM_NONE && blocked!=0xffu {
                for(var j=0u;j<8u;j+=1u) {
                    if (blocked&(1u<<j))!=0u {continue;}
                    if vsm_far(r,ring.radius,level,j) {blocked|=1u<<j;}
                }
            }
            visible=32u*(8u-countOneBits(blocked));
        }
    }
    textureStore(vsm_output,p,vec4<u32>(visible));
}
