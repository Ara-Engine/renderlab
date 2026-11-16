// N-VSM1 (docs/n-vsm1.md): `VirtualLookup::Disk`, the sun disk's visible share from the cached
// clipmap alone, with no rays, walks or per-frame sample budget. It shares virtual_shadow.wgsl's
// clipmap (top and peel layers of (triangle id, height) words, max-mips, page cache and raster)
// and replaces the page requests (vd_mark) and the lookup (vd_lookup).
//
// Method: occluder back-projection onto the light. The disk is 64 fixed sample directions, one
// bit each (the rook pattern of tools/n-vsm1/pattern.py). Every map texel near the receiver is a
// column of solid between its top surface and its underside: the peel layer when that belongs
// to the same placement (a closed mesh), otherwise the lowest neighbouring top of the same
// placement (a sheet: a sloped surface stays closed between texels, a floating card stays
// thin, and an occluder far up the light is not taken for a wall beside the receiver). A column
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
// as large per band, each a slab a few cells thick under its highest centre.
//
// Near field: a surface closer to the receiver's tangent plane than VD_BIAS texels is not
// taken from the map (its heights are a texel apart, and LOD receivers lie off the source
// surface). Nothing else answers there. N-VSM1 read the visible surface along the sun's screen
// direction for it; N-VSM2 removed that: a pixel next to the receiver is far wider than the
// disk, so every rule tried either turned lit relief black (S3's flips) or added nothing
// (docs/n-vsm1.md, "The near field").
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
// clip, previous: this and the previous frame's world-to-clip matrices (the receivers' own,
// jitter included).
// reuse (virtual_shadow_gpu.rs `reuse_on`): x 2 when nothing that sets an answer changed since
// the last frame, 1 when only the camera and the moving casters did, 0 no reuse; z the history
// half this frame writes.
// zone_lo, zone_hi, zone_light: the moving casters' swept shadow (temporal_zones.wgsl
// tr1_caster_shadow's Tr1Zones rows).
// size: the image size in pixels (xy); z: TR1's frame number plus one on reconstructed frames
// (the receiver jitter's sequence index), 0 otherwise; w: TR1's frames of history.
// above: per k in 0..=64 (two a vec4), the mask of the samples whose y rank is at least k.
// samples: the 64 sample positions in the unit disk (two a vec4), bit s first.
struct VdFrame {
    clip:mat4x4<f32>,previous:mat4x4<f32>,reuse:vec4<u32>,
    zone_lo:vec4<f32>,zone_hi:vec4<f32>,zone_light:vec4<f32>,
    size:vec4<f32>,above:array<vec4<u32>,33>,samples:array<vec4<f32>,32>,
};
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
// virtual_shadow.wgsl's sparse layout (vsm_sparse, vsm_vpage, vsm_section, vsm_page_mip).
fn vsm_sparse()->bool {return (vsm.misc.w&0x200u)!=0u;}
fn vsm_vpage(level:u32,page:vec2<u32>)->u32 {
    let n=vsm.counts.y;let o=vsm.levels[level].info.zw;
    return (level*n+((page.y+o.y)&(n-1u)))*n+((page.x+o.x)&(n-1u));
}
fn vsm_section(k:u32)->u32 {return k*VSM_MAX_LEVELS*vsm.counts.y*vsm.counts.y;}
const VSM_PAGE_MIPS:u32=5461u;
fn vsm_page_mip(slot:u32,mip:u32,local:vec2<u32>)->u32 {
    let s=VSM_PAGE>>mip;
    return slot*VSM_PAGE_MIPS+(VSM_PAGE*VSM_PAGE-4u*s*s)/3u+local.y*s+local.x;
}
// The page state (sparse: section 2 maps table entries to physical pages).
@group(0) @binding(55) var<storage,read> vsm_pages:array<u32>;
// The physical page of window-relative page `page` of `level` (VSM_NONE: not resident).
fn vsm_resident(level:u32,page:vec2<u32>)->u32 {return vsm_pages[vsm_section(2u)+vsm_vpage(level,page)];}
// The physical pages of the 2x2 pages from vd_pa at level vd_pl, resolved once per level
// window by vd_map (vd_cache_pages), so the window's texel reads skip the dependent page-table
// load (-0.57 ms on Outpost without ray hardware); other reads look the table up.
var<private> vd_pl:u32;
var<private> vd_pa:vec2<u32>;
var<private> vd_ps:vec4<u32>;
fn vd_cache_pages(level:u32,a:vec2<i32>,b:vec2<i32>) {
    let n=vsm.counts.y;let pa=vec2<u32>(a)/VSM_PAGE;let pb=vec2<u32>(b)/VSM_PAGE;
    vd_pl=level;vd_pa=pa;
    // Only the pages the window spans (a 7-texel window rarely crosses a page edge); the others
    // stay VD_UNREAD, which vd_slot looks up in the table.
    let x1=pb.x>pa.x && pa.x+1u<n;let y1=pb.y>pa.y && pa.y+1u<n;
    var e=vec4<u32>(vsm_resident(level,pa),VD_UNREAD,VD_UNREAD,VD_UNREAD);
    if x1 {e.y=vsm_resident(level,pa+vec2<u32>(1u,0u));}
    if y1 {e.z=vsm_resident(level,pa+vec2<u32>(0u,1u));}
    if x1 && y1 {e.w=vsm_resident(level,pa+vec2<u32>(1u,1u));}
    vd_ps=e;
}
const VD_UNREAD:u32=0xfffffffeu;
// The word of texel `cell` (inside the window) of `level`: VSM_NONE where its page is missing
// (sparse only), which reads as an empty texel.
fn vsm_index(cell:vec2<i32>,level:u32)->u32 {
    if !vsm_sparse() {return vsm_word(cell,level);}
    let c=vec2<u32>(cell);
    let slot=vd_slot(level,c/VSM_PAGE);
    if slot==VSM_NONE {return VSM_NONE;}
    return slot*VSM_PAGE*VSM_PAGE+(c.y%VSM_PAGE)*VSM_PAGE+c.x%VSM_PAGE;
}
// The normalized top height of mip `mip` (1..7) over the cell holding texel `cell`; a missing
// page is the highest (conservative).
// The physical page of window-relative `page` of `level`: from vd_cache_pages' 2x2 when it
// holds it, else from the table.
fn vd_slot(level:u32,page:vec2<u32>)->u32 {
    let d=page-vd_pa;
    if level==vd_pl && all(d<vec2<u32>(2u)) {
        let e=select(select(vd_ps.x,vd_ps.y,d.x==1u),select(vd_ps.z,vd_ps.w,d.x==1u),d.y==1u);
        if e!=VD_UNREAD {return e;}
    }
    return vsm_resident(level,page);
}
fn vsm_mip_at(level:u32,mip:u32,cell:vec2<u32>)->f32 {
    let slot=vd_slot(level,cell/VSM_PAGE);
    if slot==VSM_NONE {return 1.0;}
    return vsm_mips[vsm_page_mip(slot,mip,(cell%VSM_PAGE)>>vec2<u32>(mip))];
}
// The widened disk slope the pages are requested for (disk plus sun hysteresis).
fn vd_slope()->f32 {return max(vsm.params.z,1e-4);}
// Max-mip bands above the coarsest level's: their windows reach VD_R << VD_MIPS of its texels,
// within the page around the receiver's.
const VD_MIPS:u32=5u;

// Level dither: with TR1 jittering (vd.size.z > 0), a receiver within VD_BAND of a level change
// takes the coarser level with a probability rising across the band, per pixel and frame, and
// TR1 averages the two levels where the 2x step was a line. The band is that fraction of an
// octave below a footprint boundary, and that fraction of the half window inside a window edge.
// Changes from level i to i + 1 for i below VD_BAND_FROM keep the plain choice; 0 turns it off.
// Off by default (rl1, 2026-11-13): the far-tail seam is the occluder height-band step in
// vd_map, not a receiver level change, so this left it unchanged while adding short dashed
// edges where a band crossed a shadow edge. Screens: RENDERLAB_VD=VD_BAND=0.125.
override VD_BAND:f32=0.0;
override VD_BAND_FROM:u32=0u;

// ---- Page requests ----
@group(0) @binding(1) var<storage,read_write> vsm_requests:array<u32>;
@group(0) @binding(7) var vsm_hit:texture_2d<u32>;
@group(0) @binding(8) var vsm_tiles:texture_2d<u32>;
@group(0) @binding(9) var vsm_world:texture_2d<f32>;
@group(0) @binding(14) var vsm_level_out:texture_storage_2d<r32uint,write>;
// The receivers the mark pass saw (virtual_shadow_gpu.rs reads it back a frame or two late):
// 0, the nearest one's distance from the eye as !bits (0: none); 1, their count; 2, those no
// level's window holds (shaded without the map); 3, the nearest of those as !bits.
@group(0) @binding(52) var<storage,read_write> vsm_reach:array<atomic<u32>,4>;
var<workgroup> vd_reach:array<atomic<u32>,4>;
// The ladder's index for absolute level k: the coarsest level not coarser than k (0 when every
// level is). The ladder is consecutive except far out, where a span of more than
// VSM_MAX_LEVELS octaves skips some (info.y holds each level's absolute level).
fn vd_first(k:f32)->u32 {
    var first=0u;
    for(var i=1u;i<vsm.counts.x;i+=1u) {
        if f32(bitcast<i32>(vsm.levels[i].info.y))<=k {first=i;}
    }
    return first;
}
// Whether the change from level i to the next may dither.
fn vd_dithers(i:u32)->bool {return i>=VD_BAND_FROM && i+1u<vsm.counts.x;}
// ---- Density (docs/design-sun-shadows.md (a)) ----
// A receiver's footprint is the smallest light-space step from its point to a screen
// neighbour's (the worst axis at output resolution): a texel no larger never spans two pixels
// along either screen axis. Under a low sun the ground's step along the sun's azimuth shrinks by
// sin(elevation), so it asks for finer texels where its shadows stretch; a neighbour across a
// silhouette steps far and does not decide. With no neighbour on a surface, the distance times
// the pixel angle.
fn vd_footprint(id:vec2<u32>,world:vec3<f32>)->f32 {
    let size=textureDimensions(vsm_world);
    let p=vsm_light(world).xy;
    var step=3.4e38;
    for(var k=0u;k<4u;k+=1u) {
        let d=vec2<i32>(select(select(0,1,k==1u),-1,k==0u),select(select(0,1,k==3u),-1,k==2u));
        let n=vec2<i32>(id)+d;
        if any(n<vec2<i32>(0)) || any(n>=vec2<i32>(size)) || textureLoad(vsm_hit,n,0).x==0u {continue;}
        step=min(step,length(vsm_light(textureLoad(vsm_world,n,0).xyz).xy-p));
    }
    if step>=3.4e38 {return length(world-vsm.eye.xyz)*vsm.eye.w;}
    return step;
}
// The finest level at about params.w texels per footprint whose window holds the receiver's
// lookup window; coarser levels are the fallback near window edges and below the ladder's finest
// level. `jitter` in [0, 1) moves the receivers in a level change's band to the coarser side (0:
// the plain choice). Returns the level and, in bits 8..15, how many levels coarser than the
// footprint asked it is (the density's shortfall: a softer edge, never a step).
fn vd_select(world:vec3<f32>,footprint:f32,jitter:f32)->u32 {
    let p=vsm_light(world);
    let x=log2(max(footprint,1e-9)/vsm.params.y)-vsm.params.w;
    var first=vd_first(floor(x));
    if vd_dithers(first) {first=vd_first(floor(x+VD_BAND*jitter));}
    let half=0.5*f32(vsm.counts.y*VSM_PAGE);
    for(var i=first;i<vsm.counts.x;i+=1u) {
        var r=(f32(VD_R)+2.0)*vsm_texel(i);
        if vd_dithers(i) {r+=VD_BAND*jitter*half*vsm_texel(i);}
        let lo=vsm.levels[i].origin.xy;let hi=lo+vec2<f32>(vsm_extent(i));
        if all(p.xy-vec2<f32>(r)>=lo) && all(p.xy+vec2<f32>(r)<hi) {
            let short=max(bitcast<i32>(vsm.levels[i].info.y)-i32(floor(x)),0);
            return i|(u32(min(short,255))<<8u);
        }
    }
    return VSM_NONE;
}
// The level a vd_select word names (VSM_NONE stays).
fn vd_level_of(word:u32)->u32 {return select(word&0xffu,VSM_NONE,word==VSM_NONE);}
// The level dither's jitter in [0, 1): interleaved gradient noise, stepped by the golden ratio
// each TR1 frame. 0 without TR1 (vd.size.z == 0) or with VD_BAND == 0.
fn vd_band_jitter(id:vec2<u32>)->f32 {
    if VD_BAND<=0.0 || vd.size.z<=0.0 {return 0.0;}
    let ign=fract(52.9829189*fract(dot(vec2<f32>(id),vec2<f32>(0.06711056,0.00583715))));
    return fract(ign+0.61803398875*vd.size.z);
}
// Each receiver requests, from its level up, the pages its lookup window can touch: the reach
// of the highest caster or VD_R texels, whichever is smaller, plus two texels. It stops at the
// level whose band holds the highest caster. The coarsest level also serves the max-mip bands:
// whole cells of the mip whose band holds the highest caster. A dithered receiver requests
// from its plain level, so both sides of its band stay cached.
@compute @workgroup_size(8,4) fn vd_mark(@builtin(global_invocation_id) id:vec3<u32>,@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    let size=textureDimensions(vsm_world);
    // A workgroup is one trace tile; an uncovered one never had hit records written.
    if textureLoad(vsm_tiles,group.xy,0).x==0u {return;}
    let inside=all(id.xy<size);
    var level=VSM_NONE;
    if inside && textureLoad(vsm_hit,vec2<i32>(id.xy),0).x!=0u {
        let world=textureLoad(vsm_world,vec2<i32>(id.xy),0).xyz;
        let footprint=vd_footprint(id.xy,world);
        let plain=vd_level_of(vd_select(world,footprint,0.0));
        level=vd_select(world,footprint,vd_band_jitter(id.xy));
        let distance=~bitcast<u32>(length(world-vsm.eye.xyz));
        atomicMax(&vd_reach[0],distance);
        atomicAdd(&vd_reach[1],1u);
        if level==VSM_NONE {atomicAdd(&vd_reach[2],1u);atomicMax(&vd_reach[3],distance);}
        if plain!=VSM_NONE {
            let p=vsm_light(world);
            let pages=vsm.counts.y;
            let reach=vd_slope()*max(vsm.depth.w-p.z,0.0);
            // The lookup's height-band dither may take a band's top from the next coarser level.
            let keep=1.0-select(0.0,VD_HBAND,vd_hband_on());
            let chosen=vd_level_of(level);
            for(var l=plain;l<vsm.counts.x;l+=1u) {
                let texel=vsm_texel(l);
                var r=min(reach,f32(VD_R)*texel)+2.0*texel;
                if l+1u==vsm.counts.x {
                    var cell=texel;
                    for(var m=0u;m<VD_MIPS && f32(VD_R)*cell*keep<reach;m+=1u) {cell*=2.0;}
                    r=max(r,(f32(VD_R)+1.0)*cell);
                }
                let page=f32(VSM_PAGE)*texel;
                let lo=vsm.levels[l].origin.xy;
                let a=vec2<u32>(clamp(floor((p.xy-vec2<f32>(r)-lo)/page),vec2<f32>(0.0),vec2<f32>(f32(pages-1u))));
                let b=vec2<u32>(clamp(floor((p.xy+vec2<f32>(r)-lo)/page),vec2<f32>(0.0),vec2<f32>(f32(pages-1u))));
                for(var y=a.y;y<=b.y;y+=1u) {
                    for(var x=a.x;x<=b.x;x+=1u) {
                        // Benign race: every writer stores the same value.
                        vsm_requests[vsm_vpage(l,vec2<u32>(x,y))]=1u;
                    }
                }
                if l>=chosen && f32(VD_R)*texel*keep>=reach {break;}
            }
        }
    }
    // The lookup reads this level and its shortfall: one rounding of vd_select for both passes.
    if inside {textureStore(vsm_level_out,vec2<i32>(id.xy),vec4<u32>(level));}
    workgroupBarrier();
    if li==0u {
        let nearest=atomicLoad(&vd_reach[0]);
        if nearest!=0u {atomicMax(&vsm_reach[0],nearest);}
        let seen=atomicLoad(&vd_reach[1]);
        if seen!=0u {atomicAdd(&vsm_reach[1],seen);}
        let none=atomicLoad(&vd_reach[2]);
        if none!=0u {atomicAdd(&vsm_reach[2],none);atomicMax(&vsm_reach[3],atomicLoad(&vd_reach[3]));}
    }
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
// receiver's tangent plane (measured along the normal).
override VD_BIAS:f32=1.0;
// The peel layer bounds a column from below where it is the top's own placement.
override VD_PEEL:bool=true;
// The least thickness of a column, in texels of its level (see vd_map). The peel layer skips the
// surfaces this close under the top (virtual_shadow.wgsl VSM_PEEL_GAP, kept equal).
override VD_THICK:f32=4.0;
// A texel's peel surface of another placement is a second occluder under the top one (two
// sheets, or a sheet over a solid); false: only the top surface's column.
override VD_PEEL2:bool=true;
// The bands above the coarsest level read one of its texels per max-mip cell (the cell is that
// texel, as a coarser level rasterized at its centres would hold); false: the cell is its
// highest texel, which widens every occluder by up to a cell.
override VD_MIP_POINT:bool=true;
// Height-band dither: each level's band (and each max-mip band above the coarsest level) hands
// its top VD_HBAND fraction to the next coarser one with a probability rising across it, per
// pixel and TR1 frame, so TR1 averages the two texel sizes where an occluder's height above the
// receiver crosses a band: the 2x step that slid along long far shadows. Only while TR1
// jitters (vd.size.z > 0) and has VD_HBAND_AFTER frames of history (vd.size.w): a cut's first
// frames keep the plain bands, since the dither shows as stipple until TR1 averages it. 0: off.
// Off by default for now (2026-11-14): on Glint's flight view the dither widens a dotted line of
// light (empty map texels along a caster's shared triangle edge) into beads. Screens and the
// later flip: RENDERLAB_VD=VD_HBAND=0.25.
override VD_HBAND:f32=0.0;
override VD_HBAND_AFTER:f32=2.0;
fn vd_hband_on()->bool {return VD_HBAND>0.0 && vd.size.z>0.0 && vd.size.w>=VD_HBAND_AFTER;}
// Screens: 1 writes lit before any work (dispatch and page work only), 2 after the receiver's
// setup, 3 skips the map windows.
override VD_SKIP:u32=0u;

// The top of level `level`'s band: the height whose reach is VD_R of its texels. The reach is
// the receiver's own (vd_px.slope: this frame's disk, not the widened one), so a band's texel
// stays a third to a sixth of the penumbra it resolves however small the sun is.
fn vd_band(level:u32)->f32 {return f32(VD_R)*vsm_texel(level)/vd_px.slope;}
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
    if vsm_sparse() {
        var h=0.0;
        if mip<=7u {
            // The four cells holding the corners (cells of mip <= 7 lie inside one page).
            let ua=vec2<u32>(a);let ub=vec2<u32>(b);
            h=max(max(vsm_mip_at(level,mip,ua),vsm_mip_at(level,mip,vec2<u32>(ub.x,ua.y))),max(vsm_mip_at(level,mip,vec2<u32>(ua.x,ub.y)),vsm_mip_at(level,mip,ub)));
        } else {
            // Above page size: the page maxima (after the pool's mips) of the pages spanned.
            let pa=vec2<u32>(a)/VSM_PAGE;let pb=min(vec2<u32>(b)/VSM_PAGE,pa+vec2<u32>(3u));
            let after=arrayLength(&vsm_mips)-VSM_MAX_LEVELS*vsm.counts.y*vsm.counts.y;
            for(var y=pa.y;y<=pb.y;y+=1u) {
                for(var x=pa.x;x<=pb.x;x+=1u) {
                    let p=vec2<u32>(x,y);
                    if vsm_resident(level,p)==VSM_NONE {h=1.0;} else {h=max(h,vsm_mips[after+vsm_vpage(level,p)]);}
                }
            }
        }
        return fma(h,vsm.depth.z,vsm.depth.x);
    }
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

fn vd_hash(input:u32)->u32 {
    var x=input;x=(x^(x>>16u))*0x7feb352du;x=(x^(x>>15u))*0x846ca68bu;return x^(x>>16u);
}
// Per-pixel state, set once by vd_prepare.
struct VdPixel {
    valid:bool,thin:bool,level:u32,
    // A coarser level stands in while the receiver's page arrives (vd_prepare).
    stand_in:bool,
    // The receiver's world point and its ray origin.
    world:vec3<f32>,origin:vec3<f32>,
    // Light space: the origin, the receiver's point, and the unit normal of the viewed side.
    p:vec3<f32>,point:vec3<f32>,normal:vec3<f32>,
    // The disk in direction space (light xy per unit of height): its centre and radius, and
    // the largest slope of its directions from the map's axis.
    centre:vec2<f32>,radius:f32,slope:f32,
    // Each band's top as a fraction of its plain top (VD_HBAND's dither; 1: plain).
    band:f32,
    // The texel filter's sub-texel offsets of this pixel and frame, one per sample group, in
    // texels of the level a column is read at (vd_filter).
    offsets:array<vec2<f32>,4>,
};
var<private> vd_px:VdPixel;
// The blocked directions so far.
var<private> vd_mask:vec2<u32>;
fn vd_full()->bool {return all(vd_mask==vec2<u32>(0xffffffffu));}

fn vd_prepare(id:vec2<u32>) {
    vd_px.valid=false;
    vd_px.stand_in=false;
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
    vd_px.band=1.0;
    if vd_hband_on() {
        // Interleaved gradient noise stepped by the golden ratio each TR1 frame.
        let ign=fract(52.9829189*fract(dot(vec2<f32>(id),vec2<f32>(0.06711056,0.00583715))));
        vd_px.band=1.0-VD_HBAND*fract(ign+0.61803398875*vd.size.z);
    }
    vd_px.thin=normal.w<0.0;
    let selected=textureLoad(vsm_level_in,p,0).x;
    vd_px.level=vd_level_of(selected);
    if VD_COUNTS && VD_PCSS && selected!=VSM_NONE {atomicAdd(&vd_counts[0],1u);}
    if VD_COUNTS && !VD_PCSS && selected!=VSM_NONE {
        atomicAdd(&vd_counts[0],1u);
        let short=(selected>>8u)&0xffu;
        if short!=0u {atomicAdd(&vd_counts[1],1u);atomicMax(&vd_counts[2],short);}
        if short>=2u {atomicAdd(&vd_counts[3],1u);}
    }
    vd_px.world=world.xyz;
    // shadow_origin (scene.wgsl): opaque receivers stay on the viewed side; thin ones move
    // toward the light.
    let toward=select(lighting.eye-world.xyz,l,vd_px.thin);
    let n=select(-normal.xyz,normal.xyz,dot(normal.xyz,toward)>=0.0);
    vd_px.origin=fma(n,vec3<f32>(world.w),world.xyz);
    vd_px.p=vsm_light(vd_px.origin);
    vd_px.point=vsm_light(world.xyz);
    // Sparse: while the receiver's page is still arriving (the per-frame raster budget after a
    // cut, or a full pool), the first coarser level that holds it.
    if vsm_sparse() && vd_px.level!=VSM_NONE {
        let side=f32(vsm.counts.y*VSM_PAGE);
        var l=vd_px.level;
        loop {
            if l>=vsm.counts.x {l=VSM_NONE;break;}
            let x=(vd_px.p.xy-vsm.levels[l].origin.xy)/vsm_texel(l);
            if all(x>=vec2<f32>(0.0)) && all(x<vec2<f32>(side)) && vsm_resident(l,vec2<u32>(x)/VSM_PAGE)!=VSM_NONE {break;}
            l+=1u;
        }
        vd_px.stand_in=l!=vd_px.level;
        vd_px.level=l;
    }
    vd_px.normal=vec3<f32>(dot(n,vsm.u.xyz),dot(n,vsm.v.xyz),dot(n,vsm.l.xyz));
    // The current sun in the map's basis: the map keeps its basis through small sun turns.
    let s=vec3<f32>(dot(l,vsm.u.xyz),dot(l,vsm.v.xyz),dot(l,vsm.l.xyz));
    vd_px.centre=s.xy/max(s.z,1e-3);
    vd_px.radius=max(shadow_views[layer].texel.z,1e-5);
    vd_px.slope=vd_px.radius+length(vd_px.centre);
    // The texel filter's offsets: the 2x2 strata of the texel around a point that moves per
    // pixel and TR1 frame (a hash, then the R2 sequence), so a frame is a stratified 4-sample box
    // filter and TR1 averages the frames into the whole box. Without TR1 the strata are centred.
    var base=vec2<f32>(0.25);
    if vd.size.z>0.0 {
        let seed=vd_hash(id.x+id.y*u32(vd.size.x));
        let shift=vec2<f32>(f32(seed>>8u),f32(vd_hash(seed^0xa511e9b3u)>>8u))*(1.0/16777216.0);
        base=fract(vec2<f32>(vd.size.z)*vec2<f32>(0.7548776662466927,0.5698402909980532)+shift);
    }
    // Constant indices: a dynamically indexed store into private memory is dropped on NVIDIA.
    vd_px.offsets[0]=fract(base)-vec2<f32>(0.5);
    vd_px.offsets[1]=fract(base+vec2<f32>(0.5,0.0))-vec2<f32>(0.5);
    vd_px.offsets[2]=fract(base+vec2<f32>(0.0,0.5))-vec2<f32>(0.5);
    vd_px.offsets[3]=fract(base+vec2<f32>(0.5,0.5))-vec2<f32>(0.5);
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
    vd_mask|=vd_column_mask(c,half,lo_in,hi);
}
fn vd_column_mask(c:vec2<f32>,half:f32,lo_in:f32,hi:f32)->vec2<u32> {
    // Below this height its nearest point is outside the cone of the disk.
    let near=max(abs(c)-vec2<f32>(half),vec2<f32>(0.0));
    let lo=max(lo_in,length(near)/vd_px.slope);
    if lo>=hi {return vec2<u32>(0u);}
    // The square at height t covers directions (c -+ half) / t: the bounding rectangle over
    // lo..hi takes each side at the height where it lies farthest out.
    let u=vec2<f32>(1.0/hi,1.0/lo);
    let a=c-vec2<f32>(half);let b=c+vec2<f32>(half);
    let lower=a*select(vec2<f32>(u.y),vec2<f32>(u.x),a>=vec2<f32>(0.0));
    let upper=b*select(vec2<f32>(u.x),vec2<f32>(u.y),b>=vec2<f32>(0.0));
    return vd_rect((lower-vd_px.centre)/vd_px.radius,(upper-vd_px.centre)/vd_px.radius);
}

// ---- Straight occluder outlines (U-CLIPMAP edges) ----
// The map holds whichever triangle covers each texel centre, so a column's square overshoots
// or falls short of an occluder's outline by up to half a texel: a staircase one texel high.
// When a page is drawn, each outline texel (an in-page neighbour empty or of another placement)
// stores its triangle's outward edge as a line (virtual_shadow.wgsl vsm_outline_line). Where a
// column would add samples without covering the whole disk (a pixel near a shadow edge), an
// outline texel's column is widened by half a texel and clipped by that line at its height: a
// straight line from the receiver through a straight edge is a straight line across the disk
// samples. Switched with the page pass by virtual_shadow_gpu.rs `edges_on` (RENDERLAB_VD
// VD_EDGES), which also sets this default.
override VD_EDGES:bool=false;

// The edge log's counters (`RENDERLAB_VD_EDGE_LOG`, virtual_shadow_gpu.rs `edge_log`): 0 the
// receivers, 1 those whose level is coarser than their footprint asked for, 2 the largest
// shortfall in levels, 3 those two or more levels short.
@group(0) @binding(62) var<storage,read_write> vd_counts:array<atomic<u32>,4>;
// Measurement (`RENDERLAB_VD=VD_COUNTS=true`): the counters run (per-pixel atomics).
override VD_COUNTS:bool=false;
// 16 bits a texel, two a word (word = the top word's index / 2): see vsm_outline_line.
@group(0) @binding(56) var<storage,read> vsm_outline:array<u32>;
// The samples whose direction (d, 1) lies on the side `s` of the plane through the receiver's
// origin with normal `np` (light space): a straight line through the disk.
fn vd_plane(np:vec3<f32>,s:f32)->vec2<u32> {
    // s (np.xy . (centre + radius g) + np.z) >= 0, as dot(n, g) >= k.
    let n=np.xy*vd_px.radius*s;
    let k=-(dot(np.xy,vd_px.centre)+np.z)*s;
    var m=vec2<u32>(0u);
    for(var i=0u;i<32u;i+=1u) {
        let g=vd.samples[i];
        let keep=vec2<f32>(dot(n,g.xy),dot(n,g.zw))>=vec2<f32>(k);
        let bits=u32(keep.x)|(u32(keep.y)<<1u);
        if i<16u {m.x|=bits<<(2u*i);} else {m.y|=bits<<(2u*i-32u);}
    }
    return m;
}
// ---- Exact outline edges (docs/design-sun-shadows.md (d)) ----
// Read as squares, an occluder's outline in a band is a staircase one texel high whose runs grow
// as the outline nears the grid's axes (Glint's rails: a step every four texels or more), and no
// box filter narrow enough to stay sharp straightens a long run. So the outline comes from the
// geometry the texel stores: its triangle's silhouette edges as the light sees them (an edge whose
// neighbour, by the index buffer's adjacency, is missing or faces away from the light). Each such
// edge is a plane through the receiver, a straight line across the disk samples (vd_plane); the
// texel's column is its square grown by half a texel and kept on the triangle's side of every
// silhouette edge. Only an edge can cut, so coverage inside the outline stays whole, and the
// half texel reaches the outline where the square fell short. A texel whose triangle has no
// silhouette edge keeps its filtered square. Every band, every level, the frame's drawn (posed)
// triangles. Off: `RENDERLAB_VD=VD_EXACT_EDGES=false`.
override VD_EXACT_EDGES:bool=true;
struct VdVec4s {data:array<vec4<f32>>};
struct VdU32s {data:array<u32>};
@group(1) @binding(0) var<storage,read> models:array<mat4x4<f32>>;
@group(1) @binding(2) var<storage,read> primary_vertices:binding_array<VdVec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> primary_indices:binding_array<VdU32s,PRIMARY_BATCHES>;
@group(1) @binding(5) var<storage,read> primary_placements:array<u32>;
// A triangle corner, light space relative to the receiver's origin.
fn vd_corner(placement:u32,batch:u32,index:u32)->vec3<f32> {
    let v=primary_vertices[batch].data[primary_indices[batch].data[index]*3u].xyz;
    return vsm_light((models[placement]*vec4<f32>(v,1.0)).xyz)-vd_px.p;
}
// The light-space normal's height of triangle `primitive` (its facing toward the light).
fn vd_facing_z(placement:u32,batch:u32,primitive:u32)->f32 {
    let a=vd_corner(placement,batch,primitive*3u);
    let b=vd_corner(placement,batch,primitive*3u+1u);
    let c=vd_corner(placement,batch,primitive*3u+2u);
    return cross(b-a,c-a).z;
}
// The disk samples on the inner side of every silhouette edge of map word `id`'s triangle, and
// whether it has one. Neighbour e lies across the edge opposite corner e (receiver_adjacency.rs).
struct VdOutline {mask:vec2<u32>,found:bool};
fn vd_outline(id:u32)->VdOutline {
    let bits=vsm.misc.x;
    let placement=(id>>bits)-1u;
    let batch=primary_placements[placement]&0x1fffffffu;
    let primitive=id&((1u<<bits)-1u);
    let info=primary_placements[arrayLength(&primary_placements)-1u-batch];
    let adjacent=(info&0x40000000u)!=0u;
    let v=array<vec3<f32>,3>(vd_corner(placement,batch,primitive*3u),vd_corner(placement,batch,primitive*3u+1u),vd_corner(placement,batch,primitive*3u+2u));
    let facing=cross(v[1]-v[0],v[2]-v[0]).z;
    var out=VdOutline(vec2<u32>(0xffffffffu),false);
    for(var e=0u;e<3u;e+=1u) {
        var silhouette=true;
        if adjacent {
            let n=primary_indices[batch].data[((info&0x3fffffffu)+primitive)*3u+e];
            if n!=0xffffffffu {silhouette=vd_facing_z(placement,batch,n)*facing<=0.0;}
        }
        if !silhouette {continue;}
        let a=v[(e+1u)%3u];let b=v[(e+2u)%3u];
        let np=cross(a,b);
        let side=dot(np,v[e]);
        if side==0.0 {continue;}
        out.mask&=vd_plane(np,sign(side));
        out.found=true;
    }
    return out;
}
// Whether a surface point (light space) is far enough above the receiver's tangent plane to
// be an occluder: `margin` metres along the normal. A thin receiver's normal is its light-facing
// one (vd_prepare), so a tilted sheet does not shadow itself.
fn vd_above(q:vec3<f32>,margin:f32)->bool {
    return dot(vd_px.normal,q-vd_px.point)>margin;
}

// The columns of one map texel, `cell` of `level`, standing for a square of side `size` centred
// `c` (light xy): a level's own texel, or the one texel read for a max-mip cell `stride` texels
// wide. `lo`..`hi` is the band of heights being served, `margin` the near-field bias.
// ---- The texel filter (docs/design-sun-shadows.md (b)) ----
// A column's square is the texel grid's, not the occluder's: read as one square per texel, an
// occluder's outline is a staircase one texel high, and a penumbra read at a band a third of its
// width is a sum of such steps. So every column is read at four sub-texel offsets of its own
// level's grid (vd_px.offsets), each for one group of 16 disk samples (sample s in group s % 4):
// the count is the disk integral of the occluders box-filtered by one texel of the level each
// is read at, on every receiver and in every band. A staircase box-filtered by its step is a
// straight edge. The tangent-plane and bias tests keep the receiver's own point, so the offsets
// cause no acne on any slope.
const VD_GROUPS=array<u32,4>(0x11111111u,0x22222222u,0x44444444u,0x88888888u);
// Measurement: the filter's width in texels (`RENDERLAB_VD=VD_FILTER=0` reads the plain squares).
override VD_FILTER:f32=1.0;
fn vd_filtered(d:vec2<f32>,size_in:f32,lo:f32,hi:f32)->vec2<u32> {
    let h=0.5*size_in;
    let size=VD_FILTER*size_in;
    return (vd_column_mask(d-vd_px.offsets[0]*size,h,lo,hi)&vec2<u32>(VD_GROUPS[0]))
        |(vd_column_mask(d-vd_px.offsets[1]*size,h,lo,hi)&vec2<u32>(VD_GROUPS[1]))
        |(vd_column_mask(d-vd_px.offsets[2]*size,h,lo,hi)&vec2<u32>(VD_GROUPS[2]))
        |(vd_column_mask(d-vd_px.offsets[3]*size,h,lo,hi)&vec2<u32>(VD_GROUPS[3]));
}
fn vd_texel(cell:vec2<i32>,level:u32,stride:i32,c:vec2<f32>,size:f32,lo_in:f32,hi:f32,margin:f32) {
    let i=vsm_index(cell,level);
    if i==VSM_NONE {return;}
    let top=vsm_top_words[i];
    if top.x==0u {return;}
    let p=vd_px.p;
    let height=vsm_height(top);
    let t=height-p.z;
    let lo=lo_in;
    // The peel surface is under the top one: below the band or the bias when the top is.
    if t<=lo {return;}
    // A column part reaches the disk's cone only above its nearest point's distance / slope,
    // so a top under that (and the peel under it) adds nothing: skip its other loads. The texel
    // filter moves a column by up to half its size; with VD_EDGES an outline column reaches half
    // a texel further (its line clip).
    let reach=select(1.0,1.5,VD_EDGES&&stride==1)*size;
    let near=max(abs(c-p.xy)-vec2<f32>(reach),vec2<f32>(0.0));
    if length(near)>=vd_px.slope*min(t,hi) {return;}
    if !vd_above(vec3<f32>(c,height),margin) {return;}
    let side=i32(vsm.counts.y*VSM_PAGE);
    let bits=vsm.misc.x;
    // The underside. A closed mesh gives it: the peel layer, when that is the top's own
    // placement. Otherwise the top is a sheet (a card, a terrain): it is as thick as its step to
    // its lowest neighbour of the same placement, which closes a sloped surface between texels
    // and leaves a floating sheet thin. Either way a column is at least VD_THICK texels thick,
    // so the columns of a thin or oblique surface overlap in height where thinner ones would
    // leave slits of light.
    var bottom=t-VD_THICK*size;
    var peel=vec2<u32>(0u);
    if VD_PEEL {peel=vsm_peel_words[i];}
    let own=peel.x!=0u && peel.x>>bits==top.x>>bits;
    if own {
        bottom=min(bottom,vsm_height(peel)-p.z);
    } else {
        for(var k=0u;k<4u;k+=1u) {
            let n=cell+stride*vec2<i32>(select(select(0,1,k==1u),-1,k==0u),select(select(0,1,k==3u),-1,k==2u));
            if any(n<vec2<i32>(0)) || any(n>=vec2<i32>(side)) {continue;}
            let j=vsm_index(n,level);
            if j==VSM_NONE {continue;}
            let other=vsm_top_words[j];
            if other.x!=0u && other.x>>bits==top.x>>bits {bottom=min(bottom,vsm_height(other)-p.z);}
        }
    }
    let clo=max(max(bottom,lo),1e-6*size);let chi=min(t,hi);
    var m=vd_filtered(c-p.xy,size,clo,chi);
    // An outline texel: its square grown by half a texel, cut by its triangle's silhouette edges.
    if VD_EXACT_EDGES {
        let grown=vd_column_mask(c-p.xy,size,clo,chi);
        if any(grown!=vec2<u32>(0u)) && any(grown!=vec2<u32>(0xffffffffu)) {
            let outline=vd_outline(top.x);
            if outline.found {m=grown&outline.mask;}
        }
    }
    // Straight outlines: only a column that adds samples without covering the disk reads its
    // texel's line (one word), and only an outline texel's is clipped.
    if VD_EDGES && stride==1 && any((m&~vd_mask)!=vec2<u32>(0u)) && any(m!=vec2<u32>(0xffffffffu)) {
        let w=(vsm_outline[i>>1u]>>((i&1u)*16u))&0xffffu;
        if (w&0x8000u)!=0u {
            let angle=f32(w&127u)*(6.28318531/128.0)-3.14159265;
            let n=vec2<f32>(cos(angle),sin(angle));
            // The centre's signed distance from the edge (outward positive), metres.
            let rho=(f32((w>>7u)&31u)/16.0-1.0)*size;
            // Inside the edge where n . (x - c) + rho <= 0, at the texel's height t.
            m=vd_column_mask(c-p.xy,size,clo,chi)&vd_plane(vec3<f32>(n*t,dot(n,p.xy-c)+rho),-1.0);
        }
    }
    vd_mask|=m;
    // Another placement's surface under the top one: a second sheet, or the solid a sheet lies
    // over. Its own underside is unknown: the least thickness.
    if VD_PEEL2 && peel.x!=0u && !own {
        let under=vsm_height(peel);
        let t2=under-p.z;
        if t2>lo && vd_above(vec3<f32>(c,under),margin) {
            vd_mask|=vd_filtered(c-p.xy,size,max(max(t2-VD_THICK*size,lo),1e-6*size),min(t2,hi));
        }
    }
}

// The map's columns: each level's band of heights, the highest band first (vd_map).
fn vd_map() {
    vd_pl=VSM_NONE;
    let p=vd_px.p;
    let slope=vd_px.slope;
    let side=i32(vsm.counts.y*VSM_PAGE);
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
        if vsm_sparse() {vd_cache_pages(l,a,b);}
        ceiling=clamp(vd_mip_max(l,a,b)-p.z,0.0,ceiling);
    }
    // The bias is the receiver's: a coarser level's texel centres lie on the source surface as
    // exactly as its own level's, and a bias growing with the level would free the occluders of
    // a grazing receiver's whole visible sliver of the disk.
    let margin=VD_BIAS*vsm_texel(vd_px.level);
    // Bands from the highest down: a pixel whose disk fills at a high occluder's band (umbra)
    // never walks the fine bands below it. The mask is an OR, so the order changes no count.
    let texel_last=vsm_texel(last);
    let origin_last=vsm.levels[last].origin.xy;
    for(var k=0u;k<VD_MIPS;k+=1u) {
        let mip=VD_MIPS-k;
        let cell=texel_last*f32(1u<<mip);
        let lo=f32(VD_R)*0.5*cell/slope*vd_px.band;
        if lo>=ceiling {continue;}
        let hi=select(f32(VD_R)*cell/slope*vd_px.band,3.4e38,mip==VD_MIPS);
        let s=side>>mip;
        let ps=p.xy;
        let x=(ps-origin_last)/cell;
        let r=min(slope*min(ceiling,hi)/cell,f32(VD_R))+0.5;
        let a=max(vec2<i32>(floor(x-vec2<f32>(r))),vec2<i32>(0));
        let b=min(vec2<i32>(floor(x+vec2<f32>(r))),vec2<i32>(s-1));
        let base=vsm_mip_base(last,mip);
        let step=i32(1u<<mip);
        for(var y=a.y;y<=b.y;y+=1) {
            for(var cx=a.x;cx<=b.x;cx+=1) {
                var stored=0.0;
                if vsm_sparse() {stored=vsm_mip_at(last,mip,vec2<u32>(vec2<i32>(cx,y)<<vec2<u32>(mip)));}
                else {stored=vsm_mips[base+u32(y*s+cx)];}
                let height=fma(stored,vsm.depth.z,vsm.depth.x);
                let t=height-p.z;
                if t<=lo {continue;}
                if VD_MIP_POINT {
                    let at=vec2<i32>(cx,y)*step+vec2<i32>(step/2);
                    let c=origin_last+(vec2<f32>(at)+vec2<f32>(0.5))*texel_last;
                    if t*slope<=length(max(abs(c-ps)-vec2<f32>(cell),vec2<f32>(0.0))) {continue;}
                    vd_texel(at,last,step,c,cell,lo,hi,margin);
                    continue;
                }
                let c=origin_last+(vec2<f32>(f32(cx),f32(y))+vec2<f32>(0.5))*cell;
                if !vd_above(vec3<f32>(c,height),margin) {continue;}
                vd_mask|=vd_filtered(c-ps,cell,max(max(lo,t-VD_THICK*cell),1e-6*texel_last),min(t,hi));
            }
        }
        if vd_full() {return;}
    }
    for(var k=0u;k<=last-vd_px.level;k+=1u) {
        let level=last-k;
        var lo=0.0;
        if level>vd_px.level {lo=vd_band(level-1u)*vd_px.band;}
        if lo>=ceiling {continue;}
        let texel=vsm_texel(level);
        let hi=vd_band(level)*vd_px.band;
        let origin=vsm.levels[level].origin.xy;
        let x=(p.xy-origin)/texel;
        let r=min(slope*min(ceiling,hi)/texel,f32(VD_R))+0.5;
        let a=max(vec2<i32>(floor(x-vec2<f32>(r))),vec2<i32>(0));
        let b=min(vec2<i32>(floor(x+vec2<f32>(r))),vec2<i32>(side-1));
        var skip=any(b<a);
        // The window's pages first: the mip test and the texels read them from the cache.
        if !skip && vsm_sparse() {vd_cache_pages(level,a,b);}
        if !skip && (b.x-a.x)*(b.y-a.y)>=4 {skip=vd_mip_max(level,a,b)-p.z<=lo;}
        if !skip {
            for(var y=a.y;y<=b.y;y+=1) {
                for(var cx=a.x;cx<=b.x;cx+=1) {
                    vd_texel(vec2<i32>(cx,y),level,1,origin+(vec2<f32>(f32(cx),f32(y))+vec2<f32>(0.5))*texel,texel,lo,hi,margin);
                }
                if vd_full() {return;}
            }
        }
    }
}

// ---- PCSS on the clipmap (`RENDERLAB_VD_LOOKUP=pcss`, virtual_shadow_gpu.rs `pcss_on`) ----
// The industry lookup, as HDRP ships it, on our map: each level is a plain depth map of its top
// layer (no peel layer, no columns: anything nearer the sun along its direction shadows). A
// blocker search over a disk of taps around the receiver gives the average blocker distance; the
// sun's angular radius turns it into a penumbra; a disk of bilinear percentage-closer taps over
// max(penumbra radius, VD_PCSS_MIN texels) at the level whose texel is about half that radius
// filters it, so the answer is a continuous function of the receiver's position. Each texel is
// compared against the receiver's tangent plane plus VD_BIAS texels of the level read
// (receiver-plane depth bias), so a wide kernel neither self-shadows nor floats at a contact.
// Taps are a Vogel disk rotated per pixel and TR1 frame; reconstruction averages them.
override VD_PCSS:bool=false;
override VD_PCSS_SEARCH:u32=16u;
override VD_PCSS_TAPS:u32=32u;
override VD_PCSS_MIN:f32=1.5;
// Measurement: 1 draws why each receiver got its answer: 0 the ceiling (no caster above it),
// 128 the blocker search (no blocker found), else the filter's answer.
override VD_PCSS_DIAG:u32=0u;
// Whether page `page` of `level` holds this window's depths: resident (sparse pool) or
// rasterized at the level's current epoch (dense windows). PCSS reads no other page: a missing
// page would read as empty, a lit hole (vd_level_for moves coarser instead).
fn vd_page_ok(level:u32,page:vec2<u32>)->bool {
    if vsm_sparse() {return vd_slot(level,page)!=VSM_NONE;}
    return vsm_pages[vsm_vpage(level,page)]==vsm.levels[level].info.x;
}
// Whether the pages under the square q +- r of `level` are all valid (r is at most a few
// texels, so the square spans at most 2x2 pages).
fn vd_square_ok(level:u32,q:vec2<f32>,r:f32)->bool {
    let texel=vsm_texel(level);
    let origin=vsm.levels[level].origin.xy;
    let side=f32(vsm.counts.y*VSM_PAGE);
    let a=(q-vec2<f32>(r)-origin)/texel;let b=(q+vec2<f32>(r)-origin)/texel;
    if any(a<vec2<f32>(0.0)) || any(b>=vec2<f32>(side)) {return false;}
    let pa=vec2<u32>(a)/VSM_PAGE;let pb=vec2<u32>(b)/VSM_PAGE;
    return vd_page_ok(level,pa) && vd_page_ok(level,vec2<u32>(pb.x,pa.y)) && vd_page_ok(level,vec2<u32>(pa.x,pb.y)) && vd_page_ok(level,pb);
}
// Point tap at level `level`: whether the texel under light-space point q occludes the
// receiver (above its tangent plane by `margin`), and its height above the receiver.
fn vd_tap(level:u32,q:vec2<f32>,margin:f32)->vec2<f32> {
    let texel=vsm_texel(level);
    let origin=vsm.levels[level].origin.xy;
    let side=i32(vsm.counts.y*VSM_PAGE);
    let cell=vec2<i32>(floor((q-origin)/texel));
    if any(cell<vec2<i32>(0)) || any(cell>=vec2<i32>(side)) {return vec2<f32>(0.0);}
    if VD_COUNTS && !vd_page_ok(level,vec2<u32>(cell)/VSM_PAGE) {atomicAdd(&vd_counts[1],1u);}
    let i=vsm_index(cell,level);
    if i==VSM_NONE {return vec2<f32>(0.0);}
    let word=vsm_top_words[i];
    if word.x==0u {return vec2<f32>(0.0);}
    let height=vsm_height(word);
    let c=origin+(vec2<f32>(cell)+vec2<f32>(0.5))*texel;
    if !vd_above(vec3<f32>(c,height),margin) {return vec2<f32>(0.0);}
    return vec2<f32>(1.0,height-vd_px.p.z);
}
// Bilinear percentage-closer tap: the four texels around q, each compared at its own centre.
fn vd_pcf(level:u32,q:vec2<f32>,margin:f32)->f32 {
    let texel=vsm_texel(level);
    let origin=vsm.levels[level].origin.xy;
    let x=(q-origin)/texel-vec2<f32>(0.5);
    let f=x-floor(x);
    let b=origin+(floor(x)+vec2<f32>(0.5))*texel;
    let o00=vd_tap(level,b,margin).x;
    let o10=vd_tap(level,b+vec2<f32>(texel,0.0),margin).x;
    let o01=vd_tap(level,b+vec2<f32>(0.0,texel),margin).x;
    let o11=vd_tap(level,b+vec2<f32>(texel),margin).x;
    return mix(mix(o00,o10,f.x),mix(o01,o11,f.x),f.y);
}
// The finest level from `start` up whose texel is at least `texel` and whose valid pages hold
// the square q +- r (plus the bilinear tap's texel); VSM_NONE when no level does.
fn vd_level_for(start:u32,texel:f32,q:vec2<f32>,r:f32)->u32 {
    var l=start;
    while l+1u<vsm.counts.x && vsm_texel(l)<texel {l+=1u;}
    loop {
        if vd_square_ok(l,q,r+vsm_texel(l)) {return l;}
        if l+1u>=vsm.counts.x {return VSM_NONE;}
        l+=1u;
    }
    return l;
}
fn vd_pcss(id:vec2<u32>)->f32 {
    let p=vd_px.p;
    let slope=vd_px.slope;
    let last=vsm.counts.x-1u;
    // The highest caster the sun's cone can meet (vd_map's ceiling).
    var ceiling=max(vsm.depth.w-p.z,0.0);
    let side=i32(vsm.counts.y*VSM_PAGE);
    for(var i=0u;i<2u;i+=1u) {
        var l=vd_px.level;
        while l<last && vd_band(l)<ceiling {l+=1u;}
        let texel=vsm_texel(l);
        let x=(p.xy-vsm.levels[l].origin.xy)/texel;
        let r=min(slope*ceiling/texel,f32(VD_R<<VD_MIPS))+1.0;
        let a=vec2<i32>(floor(x-vec2<f32>(r)));let b=vec2<i32>(floor(x+vec2<f32>(r)));
        if any(a<vec2<i32>(0)) || any(b>=vec2<i32>(side)) {break;}
        // Dense windows: mips of a page not rasterized this epoch are stale (keep the ceiling).
        let texel_r=texel*r;
        if !vsm_sparse() && !vd_square_ok(l,p.xy,texel_r) {break;}
        if vsm_sparse() {vd_cache_pages(l,a,b);}
        ceiling=clamp(vd_mip_max(l,a,b)-p.z,0.0,ceiling);
    }
    let texel0=vsm_texel(vd_px.level);
    if ceiling<=0.0 {
        if VD_COUNTS {atomicAdd(&vd_counts[2],1u);}
        return select(1.0,0.0,VD_PCSS_DIAG==1u);
    }
    // Per pixel and frame rotation of the tap disks.
    let seed=vd_hash(id.x+id.y*u32(vd.size.x));
    let turn=fract(f32(seed>>8u)*(1.0/16777216.0)+0.61803398875*vd.size.z)*6.28318531;
    // Blocker search: radius the sun's reach at the ceiling, at a level a quarter of it.
    let search=max(vd_px.radius*ceiling,VD_PCSS_MIN*texel0);
    let centre_search=p.xy+vd_px.centre*0.5*ceiling;
    let ls=vd_level_for(vd_px.level,0.25*search,centre_search,search);
    if ls==VSM_NONE {if VD_COUNTS {atomicAdd(&vd_counts[1],1u);} return 1.0;}
    let ms=VD_BIAS*vsm_texel(ls);
    var found=0.0;var depth=0.0;
    for(var k=0u;k<VD_PCSS_SEARCH;k+=1u) {
        let r=sqrt((f32(k)+0.5)/f32(VD_PCSS_SEARCH));
        let a=f32(k)*2.39996323+turn;
        let t=vd_tap(ls,centre_search+search*r*vec2<f32>(cos(a),sin(a)),ms);
        found+=t.x;depth+=t.x*t.y;
    }
    if found==0.0 {
        if VD_COUNTS {atomicAdd(&vd_counts[3],1u);}
        return select(1.0,0.5,VD_PCSS_DIAG==1u);
    }
    let blocker=depth/found;
    // The penumbra's radius from the average blocker distance; at least VD_PCSS_MIN texels.
    let radius=max(vd_px.radius*blocker,VD_PCSS_MIN*texel0);
    let centre=p.xy+vd_px.centre*blocker;
    let lf=vd_level_for(vd_px.level,0.5*radius,centre,radius);
    if lf==VSM_NONE {if VD_COUNTS {atomicAdd(&vd_counts[1],1u);} return 1.0;}
    let mf=VD_BIAS*vsm_texel(lf);
    var occluded=0.0;
    for(var k=0u;k<VD_PCSS_TAPS;k+=1u) {
        let r=sqrt((f32(k)+0.5)/f32(VD_PCSS_TAPS));
        let a=f32(k)*2.39996323+turn;
        occluded+=vd_pcf(lf,centre+radius*r*vec2<f32>(cos(a),sin(a)),mf);
    }
    return 1.0-occluded/f32(VD_PCSS_TAPS);
}
fn vd_pixel(id:vec2<u32>)->u32 {
    if VD_SKIP==1u {return 256u;}
    vd_prepare(id);
    if !vd_px.valid || VD_SKIP==2u {return 256u;}
    vd_mask=vec2<u32>(0u);
    if !vd_px.thin && !vd_facing() {return 0u;}
    if VD_PCSS {
        // The samples under the receiver's tangent plane stay analytic (vd_facing).
        let plane=1.0-f32(countOneBits(vd_mask.x)+countOneBits(vd_mask.y))/64.0;
        if vd_px.level==VSM_NONE {return u32(round(256.0*plane));}
        return u32(round(256.0*plane*vd_pcss(id)));
    }
    let facing=vd_mask;
    if vd_px.level!=VSM_NONE && VD_SKIP!=3u {vd_map();}
    // 64 samples to the 0..256 count the shading programs read.
    let answer=4u*(64u-countOneBits(vd_mask.x)-countOneBits(vd_mask.y));
    if VD_CERTAIN && vd_px.level!=VSM_NONE {vd_certainty(facing);}
    return answer;
}
// Measurement (`RENDERLAB_VD=VD_CERTAIN=true`, with RENDERLAB_VD_EDGE_LOG): whether the map
// could classify this pixel with certainty, by its occluders alone (the samples under the
// receiver's tangent plane are analytic: no ray needs them). Classes: 0 no occluder sample, 1
// occluders and the plane cover the disk, 2 partial. vd_counts 0: receivers; 1: partial; 2: one
// class here but not the same with the receiver moved two texels of its level along either axis;
// 3: certain (one class within that margin).
override VD_CERTAIN:bool=false;
fn vd_class(facing:vec2<u32>)->u32 {
    let occluders=vd_mask&~facing;
    if all(occluders==vec2<u32>(0u)) {return 0u;}
    if all(vd_mask==vec2<u32>(0xffffffffu)) {return 1u;}
    return 2u;
}
fn vd_certainty(facing:vec2<u32>) {
    atomicAdd(&vd_counts[0],1u);
    let kind=vd_class(facing);
    if kind==2u {atomicAdd(&vd_counts[1],1u);return;}
    let p=vd_px.p;let point=vd_px.point;
    let m=2.0*vsm_texel(vd_px.level);
    var same=true;
    for(var k=0u;k<4u && same;k+=1u) {
        let d=vec2<f32>(select(select(0.0,m,k==1u),-m,k==0u),select(select(0.0,m,k==3u),-m,k==2u));
        // Along the receiver's tangent plane, so the moved point stays on its surface.
        let nl=vd_px.normal;
        let dz=select(0.0,-dot(nl.xy,d)/nl.z,nl.z>0.2);
        vd_px.p=p+vec3<f32>(d,dz);vd_px.point=point+vec3<f32>(d,dz);
        vd_mask=vec2<u32>(0u);
        if !vd_px.thin {let _f=vd_facing();}
        let moved=vd_mask;
        vd_map();
        same=vd_class(moved)==kind;
    }
    vd_px.p=p;vd_px.point=point;
    atomicAdd(&vd_counts[select(2u,3u,same)],1u);
}
// ---- Answer reuse (virtual_shadow_gpu.rs `reuse_on`) ----
// The sun's answer at a world point does not depend on the camera. Per pixel and frame the
// history holds the answer (bits 0..8), how many walks in a row gave that same fully lit or
// fully shadowed answer (bits 9..15; 0 for a partial one or a coarser level's stand-in while a
// page arrives) and the point's clip w (bits 16..31, f16), in two halves ping-ponged by frame.
// A pixel whose point was on screen last frame at the same depth, out of the moving casters'
// swept shadow, whose own and 3x3 neighbours' answers there are one class settled over
// VD_REUSE_WALKS walks, keeps it without a walk: only penumbrae, edges, disocclusions, the
// moving casters' shadows and classes the receiver jitter still moves walk. While the camera
// moves every trace tile still walks one frame in VD_REUSE_REFRESH, so an answer that should change
// is found within that many frames; on frames where nothing changed none need to.
override VD_REUSE_WALKS:u32=4u;
override VD_REUSE_REFRESH:u32=16u;
@group(0) @binding(63) var<storage,read_write> vd_history:array<u32>;
// The moving casters' swept shadow (temporal_zones.wgsl tr1_caster_shadow, the same test).
fn vd_moving_shadow(x:vec3<f32>)->bool {
    if vd.zone_lo.w==0.0 {return false;}
    let lo=vd.zone_lo.xyz;let hi=vd.zone_hi.xyz;
    if vd.zone_hi.w==0.0 {return all(x>=lo) && all(x<=hi);}
    let reach=length(max(abs(x-lo),abs(x-hi)));
    let grow=vec3<f32>(reach*vd.zone_light.w);
    let rcp=vec3<f32>(1.0)/vd.zone_light.xyz;
    let a=(lo-grow-x)*rcp;let b=(hi+grow-x)*rcp;
    let t0=min(a,b);let t1=max(a,b);
    return min(min(t1.x,t1.y),t1.z)>=max(max(max(t0.x,t0.y),t0.z),0.0);
}
fn vd_walks(h:u32)->u32 {return (h>>9u)&127u;}
fn vd_depth(h:u32)->f32 {return unpack2x16float(h>>16u).x;}
// The settled class of the 3x3 around `at` in the previous half, or VSM_NONE.
fn vd_settled(at:vec2<u32>,size:vec2<u32>,base:u32)->u32 {
    let answer=vd_history[base+at.y*size.x+at.x]&0x1ffu;
    let lo=vec2<u32>(max(vec2<i32>(at)-vec2<i32>(1),vec2<i32>(0)));
    let hi=min(at+vec2<u32>(1u),size-vec2<u32>(1u));
    for(var y=lo.y;y<=hi.y;y+=1u) {
        for(var x=lo.x;x<=hi.x;x+=1u) {
            let h=vd_history[base+y*size.x+x];
            if (h&0x1ffu)!=answer || vd_walks(h)<VD_REUSE_WALKS {return VSM_NONE;}
        }
    }
    return answer;
}
@compute @workgroup_size(8,4) fn vd_lookup(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_id) local:vec3<u32>) {
    let size=textureDimensions(vsm_world);
    let id=group.xy*VSM_TRACE_TILE+local.xy;
    if any(id>=size) || textureLoad(vsm_tiles,group.xy,0).x==0u {return;}
    let pixels=size.x*size.y;
    let write=vd.reuse.z*pixels+id.y*size.x+id.x;
    let read=(1u-vd.reuse.z)*pixels;
    let hit=textureLoad(vsm_hit,vec2<i32>(id),0).x!=0u;
    let world=textureLoad(vsm_world,vec2<i32>(id),0).xyz;
    let depth=(vd.clip*vec4<f32>(world,1.0)).w;
    let depth_bits=pack2x16float(vec2<f32>(select(0.0,depth,hit),0.0))<<16u;
    // The point's history last frame: its pixel then, when on screen at the same depth.
    var before=0u;
    var at=vec2<u32>(VSM_NONE);
    if vd.reuse.x!=0u && hit {
        if vd.reuse.x==2u {at=id;}
        else {
            let c=vd.previous*vec4<f32>(world,1.0);
            if c.w>0.0 {
                let f=(c.xy/c.w*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5))*vec2<f32>(size);
                if all(f>=vec2<f32>(0.0)) && all(f<vec2<f32>(size)) {
                    let q=vec2<u32>(f);
                    let h=vd_history[read+q.y*size.x+q.x];
                    if abs(vd_depth(h)-c.w)<=0.02*c.w {at=q;}
                }
            }
            if at.x!=VSM_NONE && vd_moving_shadow(world) {at=vec2<u32>(VSM_NONE);}
        }
        if at.x!=VSM_NONE {
            before=vd_history[read+at.y*size.x+at.x];
            // Per trace tile (one wave): a wave runs as long as its slowest pixel, so a refresh
            // spread over the pixels of every tile would keep every wave walking.
            let refresh=vd.reuse.x==1u && (vd_hash(group.x+group.y*0x10000u)+u32(vd.size.z))%VD_REUSE_REFRESH==0u;
            let kept=select(vd_settled(at,size,read),VSM_NONE,refresh);
            if kept!=VSM_NONE {
                textureStore(vsm_output,vec2<i32>(id),vec4<u32>(kept));
                vd_history[write]=depth_bits|(before&0xffffu);
                return;
            }
        }
    }
    let answer=vd_pixel(id);
    textureStore(vsm_output,vec2<i32>(id),vec4<u32>(answer));
    // A walk extends the settled count of the same class; a coarser level standing in for an
    // arriving page never settles.
    var walks=0u;
    if (answer==0u || answer==256u) && !vd_px.stand_in {
        walks=select(1u,min(vd_walks(before)+1u,127u),(before&0x1ffu)==answer && vd_walks(before)!=0u);
    }
    vd_history[write]=depth_bits|(walks<<9u)|answer;
}
