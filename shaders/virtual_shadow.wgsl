// N8/N8b: non-RT Interactive sun visibility (docs/non-rt-n8.md, docs/non-rt-n8b.md). No ray
// queries and no acceleration structure: each receiver's 8 Interactive directions get
//   - an exact watertight test against the receiver triangle and its vertex 1-ring, over
//     the whole ray (the existing edge adjacency, walked as vertex fans; skipped where the
//     triangle's precomputed ring horizon is below every direction), and
//   - a shadow-map march (SMRT-style) through a cached sun clipmap from the ray origin up to
//     the highest caster, through the receiver's level and every coarser one, skipping map
//     texels whose top triangle is in that 1-ring.
// Each level keeps two layers, each a surface height with its triangle id: the highest
// surface toward the sun and the highest other surface at the texel centre (a depth peel),
// so an occluder under a higher sheet stays visible. A sample whose layer surface is above
// the ray is decided exactly: a walk from that texel's triangle to the triangle under the
// ray, refined toward the predicted crossing (vsm_top_hits); no texel height blocks a ray.
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
    // height range, w: bit 0 to walk the receiver's vertex 1-ring (else its own triangle
    // only), bit 1 to peel under the top layer's own placement too (N8b `peel_own`).
    misc:vec4<u32>,
    levels:array<VsmLevel,VSM_MAX_LEVELS>,
    // Eye-relative light-space xy rectangles (min.xy, max.xy) of moved casters.
    rects:array<vec4<f32>,VSM_MAX_RECTS>,
    // Per traced batch, the first word of its triangles in vsm_horizons (N8b).
    horizon:array<vec4<u32>,16>,
};
@group(0) @binding(0) var<uniform> vsm:VsmFrame;

fn vsm_light(world:vec3<f32>)->vec3<f32> {
    let rel=world-vsm.eye.xyz;
    return vec3<f32>(dot(rel,vsm.u.xyz),dot(rel,vsm.v.xyz),dot(world,vsm.l.xyz));
}
fn vsm_texel(level:u32)->f32 {return vsm.levels[level].origin.z;}
fn vsm_extent(level:u32)->f32 {return f32(vsm.counts.y*VSM_PAGE)*vsm_texel(level);}
// The light-space height a level's march can serve with its ±1-page neighbourhood marked:
// the disk's drift over it stays within one page.
fn vsm_cap(level:u32)->f32 {return f32(VSM_PAGE-4u)*vsm_texel(level)/max(vsm.params.z,1e-3);}
fn vsm_reach(level:u32,height:f32)->f32 {return max(0.0,min(vsm_cap(level),vsm.depth.w-height));}
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
            let pages=vsm.counts.y;
            // The march continues through coarser levels until it passes the highest caster
            // (N8b: occluders farther above than a level's reach); each needs its pages.
            for(var l=level;l<vsm.counts.x;l+=1u) {
                let r=vsm_radius(l,p.z);
                let page=f32(VSM_PAGE)*vsm_texel(l);
                let lo=vsm.levels[l].origin.xy;
                let a=vec2<u32>(clamp(floor((p.xy-vec2<f32>(r)-lo)/page),vec2<f32>(0.0),vec2<f32>(f32(pages-1u))));
                let b=vec2<u32>(clamp(floor((p.xy+vec2<f32>(r)-lo)/page),vec2<f32>(0.0),vec2<f32>(f32(pages-1u))));
                for(var y=a.y;y<=b.y;y+=1u) {
                    for(var x=a.x;x<=b.x;x+=1u) {
                        // Benign race: every writer stores the same value.
                        vsm_requests[(l*pages+y)*pages+x]=1u;
                    }
                }
                if vsm_cap(l)>=vsm.depth.w-p.z {break;}
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
// The top layer's ids, which the peel pass excludes.
@group(0) @binding(13) var vsm_top_ids:texture_2d_array<u32>;

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
// Placement + 1 and primitive, so the lookup can recognize the receiver's own 1-ring.
@fragment fn fs_vsm_caster(input:VsmCaster,@builtin(primitive_index) primitive:u32)->@location(0) u32 {
    return ((input.placement+1u)<<vsm.misc.x)|primitive;
}
// The peel layer: the highest surface other than the texel's top triangle. Only one
// triangle of a watertight surface covers a texel centre, so this is the next surface down.
@fragment fn fs_vsm_peel(input:VsmCaster,@builtin(primitive_index) primitive:u32)->@location(0) u32 {
    let id=((input.placement+1u)<<vsm.misc.x)|primitive;
    let top=textureLoad(vsm_top_ids,vec2<i32>(input.position.xy),i32(vsm_pass.level),0).x;
    if top==id {discard;}
    return id;
}

// ---- Lookup: exact 1-ring, then the map march beyond `near` ----
@group(0) @binding(15) var vsm_output:texture_storage_2d<r32uint,write>;
@group(0) @binding(16) var vsm_depth:texture_depth_2d_array;
@group(0) @binding(17) var vsm_ids:texture_2d_array<u32>;
@group(0) @binding(18) var vsm_level_in:texture_2d<u32>;
@group(0) @binding(19) var vsm_peel:texture_depth_2d_array;
@group(0) @binding(21) var vsm_peel_ids:texture_2d_array<u32>;

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
// Diagnostics (VSM_DEBUG only): the trace scripts/n8-diagnose.py classifies, two uvec4s per
// pixel. Whole-vector read-modify-writes of named components only.
//   x: ring-blocked directions (0-7) | map-blocked (8-15) | level (24-31)
//   w: ring triangles tested (0-15) | fan stopped by: step cap (16), open edge (17),
//      non-manifold corner (18)
// The second word is reserved (zero).
override VSM_DEBUG:bool=false;
// N8b: decide rays exactly below an interior surface beyond the tested neighbourhood.
override VSM_CONTINUE:bool=true;
// March samples in each level beyond the receiver's (N8b continuation).
override VSM_EXTRA_SAMPLES:u32=2u;
// N8b screens: the peel layer in the march, and the march beyond the receiver's level.
override VSM_PEEL:bool=true;
override VSM_ASCEND:bool=true;
// Cost screens: the far march at all, and its first step from the ray origin (off: from
// `near`, as N8 did).
override VSM_FAR:bool=true;
override VSM_ORIGIN_STEP:bool=true;
// The first step tests the 2x2 texel block around the ray, not only the texel under it:
// near-contact occluder edges that miss the texel centre (N8b).
override VSM_FIRST_BLOCK:bool=true;
@group(0) @binding(20) var<storage,read_write> vsm_debug:array<vec4<u32>>;
var<private> vsm_dbg:vec4<u32>;
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
fn vsm_ring(r:VsmReceiver,walk:bool)->VsmRing {
    var ring=vsm_ring_triangle(r,vsm_own,VsmRing(0u,0.0));
    var visited=1u;
    for(var c=0u;c<3u*u32(walk);c+=1u) {
        if ring.blocked==0xffu {break;}
        let p=vsm_own[c];
        let skip=vsm_own[(c+1u)%3u];
        var closed=false;
        for(var side=0u;side<2u && !closed;side+=1u) {
            var t=r.primitive;
            // Leaving across edge (c+1) shares {c, c+2}; across edge (c+2) shares {c, c+1}.
            var edge=(c+1u+side)%3u;
            var q=vsm_own[(c+2u-side)%3u];
            var step=0u;
            for(;step<VSM_FAN_STEPS;step+=1u) {
                let n=vsm_neighbour(r.batch,t,edge);
                if n==VSM_MISSING {vsm_dbg.w|=1u<<17u;break;}
                if n==r.primitive {closed=true;break;}
                var w=vsm_triangle(r.batch,n);
                let cp=vsm_corner(w,p);let cq=vsm_corner(w,q);
                if cp==3u || cq==3u || cp==cq {vsm_dbg.w|=1u<<18u;break;}
                if vsm_corner(w,skip)==3u {ring=vsm_ring_triangle(r,w,ring);visited+=1u;}
                edge=cq;q=w[3u-cp-cq];t=n;
            }
            if step==VSM_FAN_STEPS {vsm_dbg.w|=1u<<16u;}
        }
    }
    ring.radius=sqrt(ring.radius);
    vsm_dbg.w|=min(visited,0xffffu);
    return ring;
}
// ---- Ring horizons (N8b): skip the ring walk where its triangles cannot block ----
// vsm_horizon writes, per triangle and side of its winding plane, the tangent of the
// steepest elevation at which a ray leaving the triangle can reach any triangle of its vertex
// 1-ring (the fans vsm_ring walks), rounded up. The lookup skips the walk when all eight
// directions rise more steeply on the receiver's side. Elevations are invariant under
// similarity transforms only, so other placements always walk.
struct VsmHorizonPass {batch:u32,count:u32,offset:u32,pad:u32};
@group(0) @binding(22) var<uniform> vsm_horizon_pass:VsmHorizonPass;
@group(0) @binding(23) var<storage,read_write> vsm_horizon_out:array<u32>;
@group(0) @binding(24) var<storage,read> vsm_horizons:array<u32>;
override VSM_HORIZON:bool=true;
// The largest sine of elevation above the plane of normal n among directions from a point
// of triangle vsm_ht to a point of triangle vsm_hq: the norm of n's projection onto the cone
// spanned by g = q_i - t_j (the directions from T to Q). If n is in that cone (Q overhangs
// T: their projections on the plane overlap with Q above) it is 1. Otherwise the projection
// lies on a ray or a two-generator face of the cone, so the largest projection of n onto a
// single generator or a generator pair whose coefficients are non-negative is exact.
var<private> vsm_ht:array<vec3<f32>,3>;
var<private> vsm_hq:array<vec3<f32>,3>;
var<private> vsm_hg:array<vec3<f32>,9>;
fn vsm_hg_store(k:u32,g:vec3<f32>) {
    // Constant-index stores only (docs: runtime-indexed stores misbehaved on NVIDIA).
    switch k {
        case 0u {vsm_hg[0]=g;} case 1u {vsm_hg[1]=g;} case 2u {vsm_hg[2]=g;}
        case 3u {vsm_hg[3]=g;} case 4u {vsm_hg[4]=g;} case 5u {vsm_hg[5]=g;}
        case 6u {vsm_hg[6]=g;} case 7u {vsm_hg[7]=g;} default {vsm_hg[8]=g;}
    }
}
// Whether the plane projections of T and Q overlap in their interiors (separating axes on
// the six edge normals; touching along a shared edge or vertex is not overlap).
fn vsm_overlap(n:vec3<f32>)->bool {
    for(var k=0u;k<6u;k+=1u) {
        var a:vec3<f32>;var b:vec3<f32>;
        if k<3u {a=vsm_ht[k];b=vsm_ht[(k+1u)%3u];} else {a=vsm_hq[k-3u];b=vsm_hq[(k-2u)%3u];}
        let axis=cross(b-a,n);
        var t0=3.4e38;var t1=-3.4e38;var q0=3.4e38;var q1=-3.4e38;
        for(var i=0u;i<3u;i+=1u) {
            let x=dot(vsm_ht[i],axis);let y=dot(vsm_hq[i],axis);
            t0=min(t0,x);t1=max(t1,x);q0=min(q0,y);q1=max(q1,y);
        }
        let tolerance=1e-6*(abs(t0)+abs(t1)+abs(q0)+abs(q1));
        if t1<=q0+tolerance || q1<=t0+tolerance {return false;}
    }
    return true;
}
fn vsm_cone_sine(n:vec3<f32>)->f32 {
    var count=0u;
    var above=false;
    for(var i=0u;i<3u;i+=1u) {
        above=above || dot(vsm_hq[i]-vsm_ht[0],n)>0.0;
        for(var j=0u;j<3u;j+=1u) {
            let g=vsm_hq[i]-vsm_ht[j];
            if dot(g,g)>0.0 {vsm_hg_store(count,g);count+=1u;}
        }
    }
    if !above {return 0.0;}
    if vsm_overlap(n) {return 1.0;}
    var best=0.0;
    for(var a=0u;a<count;a+=1u) {
        let ga=vsm_hg[a];
        let aa=dot(ga,ga);let an=dot(ga,n);
        if an>0.0 {best=max(best,an*inverseSqrt(aa));}
        for(var b=a+1u;b<count;b+=1u) {
            let gb=vsm_hg[b];
            let ab=dot(ga,gb);let bb=dot(gb,gb);let bn=dot(gb,n);
            let det=aa*bb-ab*ab;
            if det<=1e-12*aa*bb {continue;}
            // n's projection onto span(ga, gb): coefficients from the 2x2 Gram system.
            let la=(an*bb-bn*ab)/det;let lb=(bn*aa-an*ab)/det;
            if la>=0.0 && lb>=0.0 {best=max(best,sqrt(max(la*an+lb*bn,0.0)));}
        }
    }
    return min(best,1.0);
}
// The tangent of vsm_cone_sine: what the lookup compares with its directions.
fn vsm_kappa(n:vec3<f32>)->f32 {
    let sine=vsm_cone_sine(n);
    if sine>=0.9999 {return 3.4e38;}
    return sine*inverseSqrt(1.0-sine*sine);
}
@compute @workgroup_size(64) fn vsm_horizon(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=vsm_horizon_pass.count {return;}
    let batch=vsm_horizon_pass.batch;
    let primitive=id.x;
    let own=vsm_triangle(batch,primitive);
    vsm_ht=own;
    let cross_n=cross(own[1]-own[0],own[2]-own[0]);
    var up=3.4e38;
    var down=3.4e38;
    if dot(cross_n,cross_n)>0.0 {
        let n=normalize(cross_n);
        up=0.0;down=0.0;
        // vsm_ring's fans, without its early exit.
        for(var c=0u;c<3u;c+=1u) {
            let p=own[c];
            let skip=own[(c+1u)%3u];
            var closed=false;
            for(var side=0u;side<2u && !closed;side+=1u) {
                var t=primitive;
                var edge=(c+1u+side)%3u;
                var q=own[(c+2u-side)%3u];
                for(var step=0u;step<VSM_FAN_STEPS;step+=1u) {
                    let next=vsm_neighbour(batch,t,edge);
                    if next==VSM_MISSING {break;}
                    if next==primitive {closed=true;break;}
                    let w=vsm_triangle(batch,next);
                    let cp=vsm_corner(w,p);let cq=vsm_corner(w,q);
                    if cp==3u || cq==3u || cp==cq {break;}
                    if vsm_corner(w,skip)==3u {
                        vsm_hq=w;
                        up=max(up,vsm_kappa(n));
                        down=max(down,vsm_kappa(-n));
                    }
                    edge=cq;q=w[3u-cp-cq];t=next;
                }
            }
        }
    }
    // f16 keeps three digits; the margin keeps the bound conservative after rounding.
    let packed=min(vec2<f32>(up,down)*1.01+vec2<f32>(1e-3),vec2<f32>(65504.0));
    vsm_horizon_out[vsm_horizon_pass.offset+primitive]=pack2x16float(packed);
}
// Whether all eight directions rise above the receiver triangle's ring horizon on the ray
// origin's side, so the ring walk cannot block any of them.
fn vsm_beyond_horizon(r:VsmReceiver,l:vec3<f32>)->bool {
    if !VSM_HORIZON {return false;}
    let m=models[r.slot];
    let x=m[0].xyz;let y=m[1].xyz;let z=m[2].xyz;
    let s=dot(x,x);
    if abs(dot(y,y)-s)>1e-4*s || abs(dot(z,z)-s)>1e-4*s || abs(dot(x,y))>1e-4*s || abs(dot(y,z))>1e-4*s || abs(dot(z,x))>1e-4*s {return false;}
    let h=unpack2x16float(vsm_horizons[vsm.horizon[r.batch/4u][r.batch%4u]+r.primitive]);
    let a=vsm_to_world(vsm_own[0],r.slot);
    let winding=cross(vsm_to_world(vsm_own[1],r.slot)-a,vsm_to_world(vsm_own[2],r.slot)-a);
    if dot(winding,winding)==0.0 {return false;}
    let toward=select(lighting.eye-r.world,l,r.thin);
    let up=dot(winding,toward)>=0.0;
    let n=select(-normalize(winding),normalize(winding),up);
    let bound=select(h.y,h.x,up);
    for(var j=0u;j<8u;j+=1u) {
        let e=dot(vsm_dirs[j],n);
        if e<=0.0 || e*e<=bound*bound*(1.0-e*e) {return false;}
    }
    return true;
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
// Point-location walk steps per aim from the texel's triangle toward the ray (N8b), at
// the receiver's level; coarser levels' texels span more triangles.
override VSM_WALK_STEPS:u32=8u;
// Crossing refinements: walk aims after the sample's own xy.
override VSM_REFINE:u32=3u;
// The continuity rule's reach: the predicted crossing must lie this many spans of the last
// located triangle beyond the sample (0: any ray still below after the refinements).
override VSM_SPAN:f32=1.5;
// The current direction's ray origin and the exact tests' tmin (the receiver's epsilon).
var<private> vsm_ray_from:vec3<f32>;
var<private> vsm_tmin:f32;
fn vsm_triangle_hits(batch:u32,primitive:u32,placement:u32,j:u32)->bool {
    let v=vsm_triangle(batch,primitive);
    let m=models[placement];
    let o=vsm_ray_from;
    let a=vsm_permute((m*vec4<f32>(v[0],1.0)).xyz-o);
    let b=vsm_permute((m*vec4<f32>(v[1],1.0)).xyz-o);
    let c=vsm_permute((m*vec4<f32>(v[2],1.0)).xyz-o);
    return vsm_hits_sheared(vsm_shear[j],a,b,c,vsm_tmin);
}
// Whether direction j's ray crosses the surface the texel's triangle `id` belongs to (N8b).
// Scan triangles are often smaller than a texel, so the triangle rasterized at the texel
// centre need not be the one above the ray. Walk across the exact-position adjacency to the
// triangle under the sample's light-space xy (leaving each triangle across the edge the
// point is most outside of), testing every triangle visited exactly from the receiver's own
// epsilon on (an occluder of another mesh can be closer than `near`). If the sample is below
// that triangle's plane, the ray rises to it farther along its drift: predict that crossing
// xy, walk there, and repeat, so the crossing triangle itself is tested. 1: an exact hit.
// 2: after the refinements the ray is still exactly below an interior triangle and the
// predicted crossing lies beyond the triangles tested (VSM_SPAN spans of the last), so the
// surface is taken to continue there: a far or steep crossing. Never for the receiver's own
// triangle or 1-ring (tested over the whole ray), at an open edge (the ray may escape), or
// for a ray grazing a crest.
fn vsm_top_hits(r:VsmReceiver,id:u32,placement:u32,j:u32,q:vec3<f32>,step3:vec3<f32>,steps:u32)->u32 {
    vsm_ray_from=vsm_ray_origin(r,j);
    vsm_tmin=r.epsilon;
    let batch=primary_placements[placement]&0x1fffffffu;
    let m=models[placement];
    let o=vsm_ray_from;
    var t=id&((1u<<vsm.misc.x)-1u);
    var previous=VSM_MISSING;
    // The current walk aim and the ray's light-space height there.
    var aim=q.xy;
    var ray=q.z;
    var below=false;
    var span=0.0;
    for(var phase=0u;phase<=VSM_REFINE;phase+=1u) {
        var located=false;
        var height=0.0;
        var own=false;
        // Whether the walk stopped at an open edge, a fold or a degenerate projection rather
        // than running out of steps (a crossing farther than the walk reaches).
        var stopped=false;
        for(var step=0u;step<steps;step+=1u) {
            let v=vsm_triangle(batch,t);
            let a=(m*vec4<f32>(v[0],1.0)).xyz;let b=(m*vec4<f32>(v[1],1.0)).xyz;let c=(m*vec4<f32>(v[2],1.0)).xyz;
            if vsm_hits_sheared(vsm_shear[j],vsm_permute(a-o),vsm_permute(b-o),vsm_permute(c-o),vsm_tmin) {return 1u;}
            let la=vsm_light(a);let lb=vsm_light(b);let lc=vsm_light(c);
            let area=(lb.x-la.x)*(lc.y-la.y)-(lb.y-la.y)*(lc.x-la.x);
            if area==0.0 {stopped=true;break;}
            // Signed sub-areas opposite each vertex; all non-negative (after orientation) inside.
            let w=vec3<f32>((lc.x-lb.x)*(aim.y-lb.y)-(lc.y-lb.y)*(aim.x-lb.x),
                (la.x-lc.x)*(aim.y-lc.y)-(la.y-lc.y)*(aim.x-lc.x),
                (lb.x-la.x)*(aim.y-la.y)-(lb.y-la.y)*(aim.x-la.x))*sign(area);
            if min(w.x,min(w.y,w.z))>=0.0 {
                located=true;
                height=dot(w,vec3<f32>(la.z,lb.z,lc.z))/(w.x+w.y+w.z);
                span=max(distance(la.xy,lb.xy),max(distance(lb.xy,lc.xy),distance(lc.xy,la.xy)));
                own=placement==r.slot && (vsm_same(v[0],vsm_own[0]) || vsm_same(v[0],vsm_own[1]) || vsm_same(v[0],vsm_own[2])
                    || vsm_same(v[1],vsm_own[0]) || vsm_same(v[1],vsm_own[1]) || vsm_same(v[1],vsm_own[2])
                    || vsm_same(v[2],vsm_own[0]) || vsm_same(v[2],vsm_own[1]) || vsm_same(v[2],vsm_own[2]));
                break;
            }
            // Leave across the edge the aim is most outside of; at a fold, where that
            // edge leads back, the other outside edge.
            var edge=0u;
            if w.y<w.x && w.y<=w.z {edge=1u;} else if w.z<w.x && w.z<w.y {edge=2u;}
            var n=vsm_neighbour(batch,t,edge);
            if n==previous {
                let e1=(edge+1u)%3u;let e2=(edge+2u)%3u;
                let other=select(e2,e1,w[e1]<w[e2]);
                n=select(VSM_MISSING,vsm_neighbour(batch,t,other),w[other]<0.0);
                if n==previous {stopped=true;break;}
            }
            if n==VSM_MISSING {stopped=true;break;}
            previous=t;t=n;
        }
        var interior=true;
        for(var e=0u;e<3u;e+=1u) {
            let n=vsm_neighbour(batch,t,e);
            interior=interior && n!=VSM_MISSING;
            if n!=VSM_MISSING && n!=previous && vsm_triangle_hits(batch,n,placement,j) {return 1u;}
        }
        // A refinement aim beyond the walk's reach keeps the previous phase's verdict.
        if !located && !stopped && phase>0u {break;}
        below=false;
        if !located || own || !interior {break;}
        let gap=height-ray;
        if gap<=0.0 {break;}
        below=true;
        // The ray reaches this plane's height after rising `gap`; its xy drifts meanwhile.
        aim+=step3.xy*(gap/step3.z);
        ray+=gap;
    }
    if VSM_CONTINUE && below && distance(aim,q.xy)>VSM_SPAN*span {return 2u;}
    return 0u;
}
// SMRT-style march of one direction: geometric steps from `near` through the receiver's
// level, then each coarser one (N8b), up to the highest caster. Step i covers the ray
// heights since the previous step, the first from the ray origin itself (N8b: contacts of
// other meshes inside `near`); its texel is a candidate when a layer surface is above that
// range (and above the receiver's own plane at the texel centre), and the candidate is
// decided exactly by vsm_top_hits. Most lit samples cost one depth load.
fn vsm_far(r:VsmReceiver,ring:f32,level0:u32,j:u32)->bool {
    let d=vsm_dirs[j];
    let p=vsm_light(vsm_ray_origin(r,j));
    let step=vec3<f32>(dot(d,vsm.u.xyz),dot(d,vsm.v.xyz),dot(d,vsm.l.xyz));
    let near=vsm.params.x;
    if step.z<=0.0 {return false;}
    // The ray parameter past every caster; at least one step, since a contact closer than
    // `near` still needs its sample.
    let clear=max((vsm.depth.w-p.z)/step.z,1.0001*near);
    let size=i32(vsm.counts.y*VSM_PAGE);
    // The origin-side receiver plane in light space, for texel-centre extrapolation.
    let toward=select(lighting.eye-r.world,d,r.thin);
    let n=select(-r.geometric,r.geometric,dot(r.geometric,toward)>=0.0);
    let nl=vec3<f32>(dot(n,vsm.u.xyz),dot(n,vsm.v.xyz),dot(n,vsm.l.xyz));
    let facing=nl.z>0.0;
    let gradient=-nl.xy/max(nl.z,0.05);
    let bias=f32(vsm.misc.z)*vsm.depth.z*5.960464477539063e-8;
    let samples=max(vsm.misc.y,1u);
    let bits=vsm.misc.x;
    var tested=0u;
    var t=near;
    var below=select(fma(step.z,near,p.z),p.z,VSM_ORIGIN_STEP);
    for(var level=level0;level<select(level0+1u,vsm.counts.x,VSM_ASCEND);level+=1u) {
    let end=min(max(vsm_cap(level)/step.z,1.0001*near),clear);
    if end<=t {continue;}
    let count=select(VSM_EXTRA_SAMPLES,samples,level==level0);
    let ratio=pow(end/t,1.0/f32(count));
    let texel=vsm_texel(level);
    let lo=vsm.levels[level].origin.xy;
    let steps=VSM_WALK_STEPS<<min(level-level0,2u);
    for(var k=0u;k<count;k+=1u) {
        t*=ratio;
        let q=fma(step,vec3<f32>(t),p);
        let start=below;
        below=q.z;
        let block=VSM_FIRST_BLOCK && level==level0 && k==0u;
        let corner=vec2<i32>(floor((q.xy-lo)/texel-vec2<f32>(0.5)));
        for(var c=0u;c<select(1u,4u,block);c+=1u) {
        let cell=select(vec2<i32>(floor((q.xy-lo)/texel)),corner+vec2<i32>(i32(c&1u),i32(c>>1u)),block);
        if any(cell<vec2<i32>(0)) || any(cell>=vec2<i32>(size)) {continue;}
        var lower=start;
        if facing {
            let centre=lo+(vec2<f32>(cell)+vec2<f32>(0.5))*texel;
            lower=max(lower,p.z+dot(gradient,centre-p.xy));
        }
        let top=fma(textureLoad(vsm_depth,cell,level,0),vsm.depth.z,vsm.depth.x);
        if top<=lower+bias {continue;}
        // The top layer, then the peel layer below it while that is above the ray too. A
        // surface hidden under its own placement is usually a fold of it, whose crossings
        // the top walk decides; `peel_own` tests those too (scan overhangs).
        var above=0u;
        for(var layer=0u;layer<select(1u,2u,VSM_PEEL);layer+=1u) {
            var id=0u;
            if layer==0u {
                id=textureLoad(vsm_ids,cell,level,0).x;
                above=id>>bits;
            } else {
                let peel=fma(textureLoad(vsm_peel,cell,level,0),vsm.depth.z,vsm.depth.x);
                if peel<=lower+bias {break;}
                id=textureLoad(vsm_peel_ids,cell,level,0).x;
                if (vsm.misc.w&2u)==0u && id>>bits==above {break;}
            }
            if id==0u {continue;}
            let placement=(id>>bits)-1u;
            // Ring members were tested exactly over the whole ray; without the ring walk
            // only the receiver's own triangle was, and its neighbours are decided here.
            if placement==r.slot && (id&((1u<<bits)-1u))==r.primitive {continue;}
            if (vsm.misc.w&1u)!=0u && placement==r.slot && distance(q.xy,p.xy)<=ring+2.0*texel && vsm_in_ring(r,id) {continue;}
            if id==tested {continue;}
            tested=id;
            if vsm_top_hits(r,id,placement,j,q,step,steps)!=0u {return true;}
        }
        }
    }
    if t>=clear {break;}
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
            let ring=vsm_ring(r,(vsm.misc.w&1u)!=0u && !vsm_beyond_horizon(r,l));
            var blocked=ring.blocked;
            let level=textureLoad(vsm_level_in,p,0).x;
            vsm_dbg.x=ring.blocked|(min(level,0xffu)<<24u);
            if VSM_FAR && level!=VSM_NONE && blocked!=0xffu {
                for(var j=0u;j<8u;j+=1u) {
                    if (blocked&(1u<<j))==0u && vsm_far(r,ring.radius,level,j) {
                        blocked|=1u<<j;
                        vsm_dbg.x|=1u<<(8u+j);
                    }
                }
            }
            visible=32u*(8u-countOneBits(blocked));
        }
    }
    textureStore(vsm_output,p,vec4<u32>(visible));
    if VSM_DEBUG {
        let index=2u*(id.x+id.y*size.x);
        vsm_debug[index]=vsm_dbg;
        vsm_debug[index+1u]=vec4<u32>(0u);
    }
}
