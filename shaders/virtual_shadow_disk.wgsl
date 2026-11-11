// N-VSM1 (docs/n-vsm1.md): `VirtualLookup::Disk`, the sun disk's visible share from the cached
// clipmap alone, with no rays, walks or per-frame sample budget. It shares virtual_shadow.wgsl's
// clipmap (top and peel layers of (triangle id, height) words, max-mips, page cache and raster)
// and replaces the page requests (vd_mark) and the lookup (vd_lookup).
//
// Method: occluder back-projection onto the light. The disk is 64 fixed sample directions, one
// bit each (the rook pattern of tools/n-vsm1/pattern.py). Every map texel near the receiver is a
// column of solid between its top surface and the surface under it (the peel layer, when that
// belongs to the same placement; otherwise down without end, as a height field). A column
// part at height t above the receiver, at light-space offset q, hides the directions q/t, a
// rectangle for a square texel; a part from t0 to t1 hides the rectangles between, taken as
// their bounding rectangle. The blocked directions of all columns are OR-ed, so overlapping
// occluders never count twice and the count is exact for the 64 samples: a still frame, a
// moving frame and the first frame after a cut cost the same and carry no sampling noise.
//
// Levels: a column part at height t reaches directions inside the disk only from offsets
// within slope * t, so level L (texel s_L) serves the heights whose reach is VD_R/2 to VD_R of
// its texels (its "band"), over at most a (2 VD_R + 1)^2 window; the next level serves the
// band above over the same texel count. The receiver's level starts at the bias height. Above
// the coarsest level's band its max-mips continue the ladder (VD_MIPS of them): cells twice
// as large per band, each a column solid from its highest centre down.
//
// Near field: a surface closer to the receiver's tangent plane than VD_BIAS texels is not
// taken from the map (its heights are a texel apart, and LOD receivers lie off the source
// surface). There the visible surface decides (vd_screen): the pixels along the sun's screen
// direction are occluder samples at their own world points, tested like map columns. A sample
// counts only when its point really lies in the receiver's cone toward the disk, so a
// foreground object that merely overlaps on screen is never taken for an occluder.
// virtual_shadow_gpu.rs prepends VD_R and owns every layout below; the
// shared structures and helpers are copies of virtual_shadow.wgsl's and must keep its layouts.

const VSM_PAGE:u32=128u;
const VSM_NONE:u32=0xffffffffu;
const VSM_MAX_LEVELS:u32=8u;
const VSM_MAX_RECTS:u32=16u;
// primary_rays.wgsl PRIMARY_TILE.
const VSM_TRACE_TILE=vec2<u32>(8u,4u);
struct VsmLevel {origin:vec4<f32>,info:vec4<u32>};
// virtual_shadow.wgsl's VsmFrame, field for field, up to the members read here.
struct VsmFrame {
    u:vec4<f32>,v:vec4<f32>,l:vec4<f32>,eye:vec4<f32>,depth:vec4<f32>,params:vec4<f32>,
    counts:vec4<u32>,misc:vec4<u32>,
    levels:array<VsmLevel,VSM_MAX_LEVELS>,
};
@group(0) @binding(0) var<uniform> vsm:VsmFrame;
// clip: this frame's world-to-clip matrix (the receivers' own, jitter included).
// size: the image size in pixels (xy).
// above: per k in 0..=64 (two a vec4), the mask of the samples whose y rank is at least k.
// samples: the 64 sample positions in the unit disk (two a vec4), bit s first.
struct VdFrame {clip:mat4x4<f32>,size:vec4<f32>,above:array<vec4<u32>,33>,samples:array<vec4<f32>,32>};
@group(0) @binding(50) var<uniform> vd:VdFrame;

fn vsm_light(world:vec3<f32>)->vec3<f32> {
    let rel=world-vsm.eye.xyz;
    return vec3<f32>(dot(rel,vsm.u.xyz),dot(rel,vsm.v.xyz),dot(world,vsm.l.xyz));
}
fn vsm_texel(level:u32)->f32 {return vsm.levels[level].origin.z;}
fn vsm_extent(level:u32)->f32 {return f32(vsm.counts.y*VSM_PAGE)*vsm_texel(level);}
fn vsm_word(cell:vec2<i32>,level:u32)->u32 {
    let side=vsm.counts.y*VSM_PAGE;
    return (level*side+u32(cell.y))*side+u32(cell.x);
}
// The widened disk slope the map was requested for (disk plus sun hysteresis).
fn vd_slope()->f32 {return max(vsm.params.z,1e-4);}
// The top of level `level`'s band: the height whose reach is VD_R of its texels.
fn vd_band(level:u32)->f32 {return f32(VD_R)*vsm_texel(level)/vd_slope();}
// Max-mip bands above the coarsest level's: their windows reach VD_R << VD_MIPS of its texels,
// within the page around the receiver's.
const VD_MIPS:u32=5u;

// ---- Page requests ----
@group(0) @binding(1) var<storage,read_write> vsm_requests:array<u32>;
@group(0) @binding(7) var vsm_hit:texture_2d<u32>;
@group(0) @binding(8) var vsm_tiles:texture_2d<u32>;
@group(0) @binding(9) var vsm_world:texture_2d<f32>;
@group(0) @binding(14) var vsm_level_out:texture_storage_2d<r32uint,write>;
// The finest level at about params.w texels per pixel whose window holds the receiver's
// lookup window; coarser levels are the fallback near window edges.
fn vd_select(world:vec3<f32>)->u32 {
    let p=vsm_light(world);
    let footprint=length(world-vsm.eye.xyz)*vsm.eye.w;
    let k=floor(log2(max(footprint,1e-9)/vsm.params.y)-vsm.params.w);
    let first=u32(clamp(k-f32(bitcast<i32>(vsm.levels[0].info.y)),0.0,f32(vsm.counts.x-1u)));
    for(var i=first;i<vsm.counts.x;i+=1u) {
        let r=(f32(VD_R)+2.0)*vsm_texel(i);
        let lo=vsm.levels[i].origin.xy;let hi=lo+vec2<f32>(vsm_extent(i));
        if all(p.xy-vec2<f32>(r)>=lo) && all(p.xy+vec2<f32>(r)<hi) {return i;}
    }
    return VSM_NONE;
}
// Each receiver requests, from its level up, the pages its lookup window can touch: the reach
// of the highest caster or VD_R texels, whichever is smaller, plus two texels. It stops at the
// level whose band holds the highest caster. The coarsest level also serves the max-mip bands:
// whole cells of the mip whose band holds the highest caster.
@compute @workgroup_size(8,4) fn vd_mark(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(vsm_world);
    if any(id.xy>=size) {return;}
    // Uncovered tiles never had hit records written; read their flag first.
    if textureLoad(vsm_tiles,id.xy/VSM_TRACE_TILE,0).x==0u {return;}
    var level=VSM_NONE;
    if textureLoad(vsm_hit,vec2<i32>(id.xy),0).x!=0u {
        let world=textureLoad(vsm_world,vec2<i32>(id.xy),0).xyz;
        level=vd_select(world);
        if level!=VSM_NONE {
            let p=vsm_light(world);
            let pages=vsm.counts.y;
            let reach=vd_slope()*max(vsm.depth.w-p.z,0.0);
            for(var l=level;l<vsm.counts.x;l+=1u) {
                let texel=vsm_texel(l);
                var r=min(reach,f32(VD_R)*texel)+2.0*texel;
                if l+1u==vsm.counts.x {
                    var cell=texel;
                    for(var m=0u;m<VD_MIPS && f32(VD_R)*cell<reach;m+=1u) {cell*=2.0;}
                    r=max(r,(f32(VD_R)+1.0)*cell);
                }
                let page=f32(VSM_PAGE)*texel;
                let lo=vsm.levels[l].origin.xy;
                let a=vec2<u32>(clamp(floor((p.xy-vec2<f32>(r)-lo)/page),vec2<f32>(0.0),vec2<f32>(f32(pages-1u))));
                let b=vec2<u32>(clamp(floor((p.xy+vec2<f32>(r)-lo)/page),vec2<f32>(0.0),vec2<f32>(f32(pages-1u))));
                for(var y=a.y;y<=b.y;y+=1u) {
                    for(var x=a.x;x<=b.x;x+=1u) {
                        // Benign race: every writer stores the same value.
                        vsm_requests[(l*pages+y)*pages+x]=1u;
                    }
                }
                if f32(VD_R)*texel>=reach {break;}
            }
        }
    }
    // The lookup reads this level: one rounding of vd_select for both passes.
    textureStore(vsm_level_out,vec2<i32>(id.xy),vec4<u32>(level));
}

// ---- Lookup ----
@group(0) @binding(10) var vsm_normal:texture_2d<f32>;
@group(0) @binding(15) var vsm_output:texture_storage_2d<r32uint,write>;
// The 64-bit maps as (id, normalized height bits) words; empty texels are zero.
@group(0) @binding(16) var<storage,read> vsm_top_words:array<vec2<u32>>;
@group(0) @binding(18) var vsm_level_in:texture_2d<u32>;
@group(0) @binding(19) var<storage,read> vsm_peel_words:array<vec2<u32>>;
@group(0) @binding(35) var<storage,read> vsm_mips:array<f32>;
struct LightingUniform {eye:vec3<f32>,count:u32,environment:vec3<f32>,exposure:f32,map:vec4<f32>};
struct Light {position_range:vec4<f32>,travel_outer:vec4<f32>,rgb_inner:vec4<f32>,kind:vec4<u32>, source:vec4<f32>};
struct ShadowView {world_to_clip:mat4x4<f32>,coverage_bias:vec4<f32>,camera_depth:vec4<f32>,texel:vec4<f32>};
@group(3) @binding(0) var<uniform> lighting:LightingUniform;
@group(3) @binding(1) var<storage,read> lights:array<Light>;
@group(3) @binding(5) var<storage,read> shadow_views:array<ShadowView>;

// Screen constants (variant screens set them; the defaults are the shipped lookup).
// A map surface blocks only when it is this many texels of the receiver's level above the
// receiver's tangent plane (measured along the normal); below that the visible surface decides.
override VD_BIAS:f32=1.0;
// The same for a visible-surface sample, in pixel footprints at the receiver.
override VD_SCREEN_BIAS:f32=0.1;
// Pixels the visible-surface samples reach along the sun's screen direction (0: off).
override VD_SCREEN:u32=6u;
// The peel layer bounds a column from below (off: every column is solid without end).
override VD_PEEL:bool=true;
// Screens: 1 writes lit before any work (dispatch and page work only), 2 after the receiver's
// setup, 3 skips the map windows, 4 the visible-surface samples.
override VD_SKIP:u32=0u;

fn vsm_height(word:vec2<u32>)->f32 {return fma(bitcast<f32>(word.y),vsm.depth.z,vsm.depth.x);}
// virtual_shadow.wgsl's max-mip layout.
fn vsm_mip_base(level:u32,mip:u32)->u32 {
    let side=vsm.counts.y*VSM_PAGE;
    let s=side>>(mip-1u);
    return level*((side*side-1u)/3u)+(side*side-s*s)/3u;
}
// The highest top-layer centre in cells a..b (inclusive, at most 16 apart) of `level`, in
// metres; over a little more than those cells, as the max-mip cells holding them.
fn vd_mip_max(level:u32,a:vec2<i32>,b:vec2<i32>)->f32 {
    let side=vsm.counts.y*VSM_PAGE;
    let span=u32(max(b.x-a.x,b.y-a.y))+1u;
    let mip=max(32u-countLeadingZeros(span-1u),1u);
    let ia=vec2<u32>(a)>>vec2<u32>(mip);let ib=vec2<u32>(b)>>vec2<u32>(mip);
    let s=side>>mip;let base=vsm_mip_base(level,mip);
    let h=max(max(vsm_mips[base+ia.y*s+ia.x],vsm_mips[base+ia.y*s+ib.x]),max(vsm_mips[base+ib.y*s+ia.x],vsm_mips[base+ib.y*s+ib.x]));
    return fma(h,vsm.depth.z,vsm.depth.x);
}

// ---- The disk as a 64-bit mask ----
// 64 sample directions in the unit disk (g: disk radii from its centre), bit s in x for s < 32
// and in y above. The samples are a rook pattern: sample s has the s-th smallest g.x, and its
// g.y rank is a permutation (vd.above). Both coordinates sit at the 64 quantiles of the disk's
// marginal distribution, so the samples on one side of an axis-aligned line are a rank, and an
// axis-aligned occluder edge crossing the disk passes one sample at a time.
// The share of the disk's area with a coordinate below g.
fn vd_share(g:f32)->f32 {
    let x=clamp(g,-1.0,1.0);
    return 0.5+(x*sqrt(1.0-x*x)+asin(x))*0.3183098861837907;
}
// Bits k..63.
fn vd_from(k:u32)->vec2<u32> {
    if k>=64u {return vec2<u32>(0u);}
    if k>=32u {return vec2<u32>(0u,0xffffffffu<<(k-32u));}
    return vec2<u32>(0xffffffffu<<k,0xffffffffu);
}
// The samples whose y rank is at least k.
fn vd_above_rank(k:u32)->vec2<u32> {
    let e=vd.above[k>>1u];
    return select(e.xy,e.zw,(k&1u)!=0u);
}
// The samples inside the rectangle g0..g1.
fn vd_rect(g0:vec2<f32>,g1:vec2<f32>)->vec2<u32> {
    if any(g1<=vec2<f32>(-1.0)) || any(g0>=vec2<f32>(1.0)) || any(g1<g0) {return vec2<u32>(0u);}
    // Sample r sits at quantile (r + 0.5) / 64: those below g0, and those up to g1.
    let first=vec2<u32>(clamp(ceil(64.0*vec2<f32>(vd_share(g0.x),vd_share(g0.y))-vec2<f32>(0.5)),vec2<f32>(0.0),vec2<f32>(64.0)));
    let end=vec2<u32>(clamp(floor(64.0*vec2<f32>(vd_share(g1.x),vd_share(g1.y))+vec2<f32>(0.5)),vec2<f32>(0.0),vec2<f32>(64.0)));
    if any(end<=first) {return vec2<u32>(0u);}
    return vd_from(first.x)&~vd_from(end.x)&vd_above_rank(first.y)&~vd_above_rank(end.y);
}

// Per-pixel state, set once by vd_prepare.
struct VdPixel {
    valid:bool,thin:bool,level:u32,
    // The receiver's world point and its ray origin.
    world:vec3<f32>,origin:vec3<f32>,
    // Light space: the origin, the receiver's point, and the unit normal of the viewed side.
    p:vec3<f32>,point:vec3<f32>,normal:vec3<f32>,
    // The disk in direction space (light xy per unit of height): its centre and radius.
    centre:vec2<f32>,radius:f32,
    // The pixel footprint at the receiver, metres.
    footprint:f32,
};
var<private> vd_px:VdPixel;
// The blocked directions so far.
var<private> vd_mask:vec2<u32>;
fn vd_full()->bool {return all(vd_mask==vec2<u32>(0xffffffffu));}

fn vd_prepare(id:vec2<u32>) {
    vd_px.valid=false;
    let p=vec2<i32>(id);
    let record=textureLoad(vsm_hit,p,0);
    let normal=textureLoad(vsm_normal,p,0);
    let light=lights[0];
    if record.x==0u || normal.w==0.0 || light.kind.z==0u {return;}
    let world=textureLoad(vsm_world,p,0);
    // fresh_estimate's cascade coverage, as the other lookups.
    let depth=dot(shadow_views[light.kind.y].camera_depth,vec4<f32>(world.xyz,1.0));
    var layer=VSM_NONE;
    for(var i=0u;i<light.kind.z;i+=1u) {
        let coverage=shadow_views[light.kind.y+i].coverage_bias;
        if depth>=coverage.x && depth<=coverage.y {layer=light.kind.y+i;break;}
    }
    if layer==VSM_NONE {return;}
    let l=-light.travel_outer.xyz;
    vd_px.valid=true;
    vd_px.thin=normal.w<0.0;
    vd_px.level=textureLoad(vsm_level_in,p,0).x;
    vd_px.world=world.xyz;
    // shadow_origin (scene.wgsl): opaque receivers stay on the viewed side; thin ones move
    // toward the light.
    let toward=select(lighting.eye-world.xyz,l,vd_px.thin);
    let n=select(-normal.xyz,normal.xyz,dot(normal.xyz,toward)>=0.0);
    vd_px.origin=fma(n,vec3<f32>(world.w),world.xyz);
    vd_px.p=vsm_light(vd_px.origin);
    vd_px.point=vsm_light(world.xyz);
    vd_px.normal=vec3<f32>(dot(n,vsm.u.xyz),dot(n,vsm.v.xyz),dot(n,vsm.l.xyz));
    // The current sun in the map's basis: the map keeps its basis through small sun turns.
    let s=vec3<f32>(dot(l,vsm.u.xyz),dot(l,vsm.v.xyz),dot(l,vsm.l.xyz));
    vd_px.centre=s.xy/max(s.z,1e-3);
    vd_px.radius=max(shadow_views[layer].texel.z,1e-5);
    vd_px.footprint=length(world.xyz-vsm.eye.xyz)*vsm.eye.w;
}

// A direction (d,1) leaves an opaque receiver's surface only above its tangent plane. Blocks
// the samples below it; false when none is above.
fn vd_facing()->bool {
    let n=vd_px.normal;
    // n . (centre + radius g, 1) > 0, as dot(a, g) + c > 0.
    let a=n.xy*vd_px.radius;
    let c=n.z+dot(n.xy,vd_px.centre);
    let reach=length(a);
    if c>=reach {return true;}
    if c<=-reach {return false;}
    // Terminator receivers only: each sample against the plane.
    for(var i=0u;i<32u;i+=1u) {
        let s=vd.samples[i];
        let low=vec2<f32>(dot(a,s.xy),dot(a,s.zw))+vec2<f32>(c)<=vec2<f32>(0.0);
        let bits=u32(low.x)|(u32(low.y)<<1u);
        if i<16u {vd_mask.x|=bits<<(2u*i);} else {vd_mask.y|=bits<<(2u*i-32u);}
    }
    return !vd_full();
}

// Blocks the directions hidden by a column part: a square of half side `half` centred `c`
// (light xy, metres) from the origin, solid from height `lo` to `hi` above it (metres).
fn vd_column(c:vec2<f32>,half:f32,lo_in:f32,hi:f32) {
    // Below this height its nearest point is outside the cone of the widened disk.
    let near=max(abs(c)-vec2<f32>(half),vec2<f32>(0.0));
    let lo=max(lo_in,length(near)/vd_slope());
    if lo>=hi {return;}
    // The square at height t covers directions (c -+ half) / t: the bounding rectangle over
    // lo..hi takes each side at the height where it lies farthest out.
    let u=vec2<f32>(1.0/hi,1.0/lo);
    let a=c-vec2<f32>(half);let b=c+vec2<f32>(half);
    let lower=a*select(vec2<f32>(u.y),vec2<f32>(u.x),a>=vec2<f32>(0.0));
    let upper=b*select(vec2<f32>(u.x),vec2<f32>(u.y),b>=vec2<f32>(0.0));
    vd_mask|=vd_rect((lower-vd_px.centre)/vd_px.radius,(upper-vd_px.centre)/vd_px.radius);
}

// Whether a surface point (light space) is far enough above the receiver's tangent plane to
// be an occluder: `margin` metres along the normal (thin receivers: in height).
fn vd_above(q:vec3<f32>,margin:f32)->bool {
    if vd_px.thin {return q.z-vd_px.p.z>margin;}
    return dot(vd_px.normal,q-vd_px.point)>margin;
}

// The map's columns: from the receiver's level up, each level's band of heights.
fn vd_map() {
    let p=vd_px.p;
    let slope=vd_slope();
    let side=i32(vsm.counts.y*VSM_PAGE);
    let bits=vsm.misc.x;
    let last=vsm.counts.x-1u;
    // The highest caster the cone can meet: the max-mip over its footprint at the level whose
    // band holds the height, refined once from the scene's highest caster.
    var ceiling=max(vsm.depth.w-p.z,0.0);
    for(var i=0u;i<2u;i+=1u) {
        var l=vd_px.level;
        while l<last && vd_band(l)<ceiling {l+=1u;}
        let texel=vsm_texel(l);
        let x=(p.xy-vsm.levels[l].origin.xy)/texel;
        let r=min(slope*ceiling/texel,f32(select(VD_R,VD_R<<VD_MIPS,l==last)))+1.0;
        let a=vec2<i32>(floor(x-vec2<f32>(r)));let b=vec2<i32>(floor(x+vec2<f32>(r)));
        if any(a<vec2<i32>(0)) || any(b>=vec2<i32>(side)) {break;}
        ceiling=clamp(vd_mip_max(l,a,b)-p.z,0.0,ceiling);
    }
    // The bias is the receiver's: a coarser level's texel centres lie on the source surface as
    // exactly as its own level's, and a bias growing with the level would free the occluders of
    // a grazing receiver's whole visible sliver of the disk.
    let margin=VD_BIAS*vsm_texel(vd_px.level);
    var lo=0.0;
    for(var level=vd_px.level;level<=last;level+=1u) {
        if lo>=ceiling {break;}
        let texel=vsm_texel(level);
        let hi=vd_band(level);
        let origin=vsm.levels[level].origin.xy;
        let x=(p.xy-origin)/texel;
        let r=min(slope*min(ceiling,hi)/texel,f32(VD_R));
        let a=max(vec2<i32>(floor(x-vec2<f32>(r))),vec2<i32>(0));
        let b=min(vec2<i32>(floor(x+vec2<f32>(r))),vec2<i32>(side-1));
        // No window, or nothing in it reaches the band.
        var skip=any(b<a);
        if !skip && (b.x-a.x)*(b.y-a.y)>=4 {skip=vd_mip_max(level,a,b)-p.z<=lo;}
        if !skip {
            for(var y=a.y;y<=b.y;y+=1) {
                for(var cx=a.x;cx<=b.x;cx+=1) {
                    let i=vsm_word(vec2<i32>(cx,y),level);
                    let top=vsm_top_words[i];
                    if top.x==0u {continue;}
                    let c=origin+(vec2<f32>(f32(cx),f32(y))+vec2<f32>(0.5))*texel;
                    let height=vsm_height(top);
                    let t=height-p.z;
                    if t<=lo || !vd_above(vec3<f32>(c,height),margin) {continue;}
                    var bottom=lo;
                    if VD_PEEL {
                        let peel=vsm_peel_words[i];
                        if peel.x!=0u && peel.x>>bits==top.x>>bits {bottom=max(lo,vsm_height(peel)-p.z);}
                    }
                    vd_column(c-p.xy,0.5*texel,max(bottom,1e-6*texel),min(t,hi));
                }
            }
            if vd_full() {return;}
        }
        lo=hi;
    }
    // Above the coarsest level's band: its max-mips, one band each; the last takes every
    // height above too, over the same window.
    let texel=vsm_texel(last);
    let origin=vsm.levels[last].origin.xy;
    for(var mip=1u;mip<=VD_MIPS;mip+=1u) {
        if lo>=ceiling {break;}
        let cell=texel*f32(1u<<mip);
        let hi=select(f32(VD_R)*cell/slope,3.4e38,mip==VD_MIPS);
        let s=side>>mip;
        let x=(p.xy-origin)/cell;
        let r=min(slope*min(ceiling,hi)/cell,f32(VD_R));
        let a=max(vec2<i32>(floor(x-vec2<f32>(r))),vec2<i32>(0));
        let b=min(vec2<i32>(floor(x+vec2<f32>(r))),vec2<i32>(s-1));
        let base=vsm_mip_base(last,mip);
        for(var y=a.y;y<=b.y;y+=1) {
            for(var cx=a.x;cx<=b.x;cx+=1) {
                let height=fma(vsm_mips[base+u32(y*s+cx)],vsm.depth.z,vsm.depth.x);
                let c=origin+(vec2<f32>(f32(cx),f32(y))+vec2<f32>(0.5))*cell;
                let t=height-p.z;
                if t<=lo || !vd_above(vec3<f32>(c,height),margin) {continue;}
                vd_column(c-p.xy,0.5*cell,max(lo,1e-6*texel),min(t,hi));
            }
        }
        if vd_full() {return;}
        lo=hi;
    }
}

// The visible surface as occluder samples: the pixels along the sun's screen direction from
// the receiver, each a column at its own world point down to where it leaves the cone.
fn vd_screen(id:vec2<u32>) {
    let l=normalize(vd_px.centre.x*vsm.u.xyz+vd_px.centre.y*vsm.v.xyz+vsm.l.xyz);
    let reach=f32(VD_SCREEN)*vd_px.footprint;
    let a=vd.clip*vec4<f32>(vd_px.origin,1.0);
    let b=vd.clip*vec4<f32>(vd_px.origin+l*reach,1.0);
    if a.w<=1e-6 || b.w<=1e-6 {return;}
    let scale=vec2<f32>(0.5,-0.5)*vd.size.xy;
    let span=(b.xy/b.w-a.xy/a.w)*scale;
    let pixels=max(abs(span.x),abs(span.y));
    // A sun along the view ray moves no pixel: nothing visible lies above the receiver.
    if pixels<0.5 {return;}
    let step=span/pixels;
    let first=vec2<f32>(id)+vec2<f32>(0.5);
    let size=vec2<i32>(textureDimensions(vsm_world));
    let margin=VD_SCREEN_BIAS*vd_px.footprint;
    for(var k=1u;k<=min(VD_SCREEN,u32(ceil(pixels)));k+=1u) {
        let s=vec2<i32>(floor(first+step*f32(k)));
        if any(s<vec2<i32>(0)) || any(s>=size) {return;}
        if textureLoad(vsm_tiles,vec2<u32>(s)/VSM_TRACE_TILE,0).x==0u || textureLoad(vsm_hit,s,0).x==0u {continue;}
        let world=textureLoad(vsm_world,s,0).xyz;
        let q=vsm_light(world);
        let t=q.z-vd_px.p.z;
        if t<=0.0 || !vd_above(q,margin) {continue;}
        vd_column(q.xy-vd_px.p.xy,0.5*length(world-vsm.eye.xyz)*vsm.eye.w,1e-6*vd_px.footprint,t);
        if vd_full() {return;}
    }
}

fn vd_pixel(id:vec2<u32>)->u32 {
    if VD_SKIP==1u {return 256u;}
    vd_prepare(id);
    if !vd_px.valid || VD_SKIP==2u {return 256u;}
    vd_mask=vec2<u32>(0u);
    if !vd_px.thin && !vd_facing() {return 0u;}
    if vd_px.level!=VSM_NONE && VD_SKIP!=3u {vd_map();}
    if VD_SCREEN!=0u && VD_SKIP!=4u && !vd_full() {vd_screen(id);}
    // 64 samples to the 0..256 count the shading programs read.
    return 4u*(64u-countOneBits(vd_mask.x)-countOneBits(vd_mask.y));
}
@compute @workgroup_size(8,4) fn vd_lookup(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_id) local:vec3<u32>) {
    let size=textureDimensions(vsm_world);
    let id=group.xy*VSM_TRACE_TILE+local.xy;
    if any(id>=size) || textureLoad(vsm_tiles,group.xy,0).x==0u {return;}
    textureStore(vsm_output,vec2<i32>(id),vec4<u32>(vd_pixel(id)));
}
