// GI receiver output: 48 bytes/pixel, two f32 scales and 39 snorm8 coefficients and a flag
// byte. A negative sky scale: no value; the flag byte (the last) set: the sky and bounce slots
// are absolute (gi_rx_absolute: a final-gather record).
// Three vector words retain the original byte layout, with vector loads/stores.
struct GiPrepared { r0:vec4<u32>, r1:vec4<u32>, r2:vec4<u32> };
@group(0) @binding(68) var<storage,read_write> gi_prepared:array<GiPrepared>;
fn gi_pass_load(input:SurfaceInput,id:vec2<u32>,size:vec2<u32>) {
    let at=id.y*size.x+id.x;
    let a=gi_prepared[at].r0;let b=gi_prepared[at].r1;let c=gi_prepared[at].r2;
    let scales=bitcast<vec2<f32>>(a.xy);
    gi_rx_valid=scales.x>=0.0;
    gi_rx_absolute=unpack4x8snorm(c.w).w>0.5;
    let bounce_scale=scales.y;
    gi_rx_normal=unit_normal(gi_bin_source(gi_viewed_normal(input.receiver.world,input.receiver.geometric),input.normal));
    gi_rx_sky[0u]=vec3<f32>(unpack4x8snorm(a.z).x,unpack4x8snorm(a.z).y,unpack4x8snorm(a.z).z)*scales.x;
    gi_rx_sky[1u]=vec3<f32>(unpack4x8snorm(a.z).w,unpack4x8snorm(a.w).x,unpack4x8snorm(a.w).y)*scales.x;
    gi_rx_sky[2u]=vec3<f32>(unpack4x8snorm(a.w).z,unpack4x8snorm(a.w).w,unpack4x8snorm(b.x).x)*scales.x;
    gi_rx_sky[3u]=vec3<f32>(unpack4x8snorm(b.x).y,unpack4x8snorm(b.x).z,unpack4x8snorm(b.x).w)*scales.x;
    gi_rx_sky[4u]=vec3<f32>(unpack4x8snorm(b.y).x,unpack4x8snorm(b.y).y,unpack4x8snorm(b.y).z)*scales.x;
    gi_rx_sky[5u]=vec3<f32>(unpack4x8snorm(b.y).w,unpack4x8snorm(b.z).x,unpack4x8snorm(b.z).y)*scales.x;
    gi_rx_sky[6u]=vec3<f32>(unpack4x8snorm(b.z).z,unpack4x8snorm(b.z).w,unpack4x8snorm(b.w).x)*scales.x;
    gi_rx_sky[7u]=vec3<f32>(unpack4x8snorm(b.w).y,unpack4x8snorm(b.w).z,unpack4x8snorm(b.w).w)*scales.x;
    gi_rx_sky[8u]=vec3<f32>(unpack4x8snorm(c.x).x,unpack4x8snorm(c.x).y,unpack4x8snorm(c.x).z)*scales.x;
    gi_rx_bounce[0u]=vec3<f32>(unpack4x8snorm(c.x).w,unpack4x8snorm(c.y).x,unpack4x8snorm(c.y).y)*bounce_scale;
    gi_rx_bounce[1u]=vec3<f32>(unpack4x8snorm(c.y).z,unpack4x8snorm(c.y).w,unpack4x8snorm(c.z).x)*bounce_scale;
    gi_rx_bounce[2u]=vec3<f32>(unpack4x8snorm(c.z).y,unpack4x8snorm(c.z).z,unpack4x8snorm(c.z).w)*bounce_scale;
    gi_rx_bounce[3u]=vec3<f32>(unpack4x8snorm(c.w).x,unpack4x8snorm(c.w).y,unpack4x8snorm(c.w).z)*bounce_scale;
}
// ---- GI receiver compute entry ----
// The prepared lookup (gi_rx_*) as pixel `at`'s 48-byte record.
fn gi_rx_store_at(at:u32) {
    var sky=0.0;var bounce=0.0;
    sky=max(sky,max(max(abs(gi_rx_sky[0u]).x,abs(gi_rx_sky[0u]).y),abs(gi_rx_sky[0u]).z));
    sky=max(sky,max(max(abs(gi_rx_sky[1u]).x,abs(gi_rx_sky[1u]).y),abs(gi_rx_sky[1u]).z));
    sky=max(sky,max(max(abs(gi_rx_sky[2u]).x,abs(gi_rx_sky[2u]).y),abs(gi_rx_sky[2u]).z));
    sky=max(sky,max(max(abs(gi_rx_sky[3u]).x,abs(gi_rx_sky[3u]).y),abs(gi_rx_sky[3u]).z));
    sky=max(sky,max(max(abs(gi_rx_sky[4u]).x,abs(gi_rx_sky[4u]).y),abs(gi_rx_sky[4u]).z));
    sky=max(sky,max(max(abs(gi_rx_sky[5u]).x,abs(gi_rx_sky[5u]).y),abs(gi_rx_sky[5u]).z));
    sky=max(sky,max(max(abs(gi_rx_sky[6u]).x,abs(gi_rx_sky[6u]).y),abs(gi_rx_sky[6u]).z));
    sky=max(sky,max(max(abs(gi_rx_sky[7u]).x,abs(gi_rx_sky[7u]).y),abs(gi_rx_sky[7u]).z));
    sky=max(sky,max(max(abs(gi_rx_sky[8u]).x,abs(gi_rx_sky[8u]).y),abs(gi_rx_sky[8u]).z));
    bounce=max(bounce,max(max(abs(gi_rx_bounce[0u]).x,abs(gi_rx_bounce[0u]).y),abs(gi_rx_bounce[0u]).z));
    bounce=max(bounce,max(max(abs(gi_rx_bounce[1u]).x,abs(gi_rx_bounce[1u]).y),abs(gi_rx_bounce[1u]).z));
    bounce=max(bounce,max(max(abs(gi_rx_bounce[2u]).x,abs(gi_rx_bounce[2u]).y),abs(gi_rx_bounce[2u]).z));
    bounce=max(bounce,max(max(abs(gi_rx_bounce[3u]).x,abs(gi_rx_bounce[3u]).y),abs(gi_rx_bounce[3u]).z));
    let ss=max(sky,1e-30);let bs=max(bounce,1e-30);
    let q0=pack4x8snorm(vec4<f32>(gi_rx_sky[0u].x,gi_rx_sky[0u].y,gi_rx_sky[0u].z,gi_rx_sky[1u].x)/vec4<f32>(ss,ss,ss,ss));
    let q1=pack4x8snorm(vec4<f32>(gi_rx_sky[1u].y,gi_rx_sky[1u].z,gi_rx_sky[2u].x,gi_rx_sky[2u].y)/vec4<f32>(ss,ss,ss,ss));
    let q2=pack4x8snorm(vec4<f32>(gi_rx_sky[2u].z,gi_rx_sky[3u].x,gi_rx_sky[3u].y,gi_rx_sky[3u].z)/vec4<f32>(ss,ss,ss,ss));
    let q3=pack4x8snorm(vec4<f32>(gi_rx_sky[4u].x,gi_rx_sky[4u].y,gi_rx_sky[4u].z,gi_rx_sky[5u].x)/vec4<f32>(ss,ss,ss,ss));
    let q4=pack4x8snorm(vec4<f32>(gi_rx_sky[5u].y,gi_rx_sky[5u].z,gi_rx_sky[6u].x,gi_rx_sky[6u].y)/vec4<f32>(ss,ss,ss,ss));
    let q5=pack4x8snorm(vec4<f32>(gi_rx_sky[6u].z,gi_rx_sky[7u].x,gi_rx_sky[7u].y,gi_rx_sky[7u].z)/vec4<f32>(ss,ss,ss,ss));
    let q6=pack4x8snorm(vec4<f32>(gi_rx_sky[8u].x,gi_rx_sky[8u].y,gi_rx_sky[8u].z,gi_rx_bounce[0u].x)/vec4<f32>(ss,ss,ss,bs));
    let q7=pack4x8snorm(vec4<f32>(gi_rx_bounce[0u].y,gi_rx_bounce[0u].z,gi_rx_bounce[1u].x,gi_rx_bounce[1u].y)/vec4<f32>(bs,bs,bs,bs));
    let q8=pack4x8snorm(vec4<f32>(gi_rx_bounce[1u].z,gi_rx_bounce[2u].x,gi_rx_bounce[2u].y,gi_rx_bounce[2u].z)/vec4<f32>(bs,bs,bs,bs));
    let q9=pack4x8snorm(vec4<f32>(gi_rx_bounce[3u].x/bs,gi_rx_bounce[3u].y/bs,gi_rx_bounce[3u].z/bs,select(0.0,1.0,gi_rx_absolute)));
    gi_prepared[at].r0=vec4<u32>(bitcast<vec2<u32>>(vec2<f32>(select(-1.0,sky,gi_rx_valid),bounce)),q0,q1);
    gi_prepared[at].r1=vec4<u32>(q2,q3,q4,q5);
    gi_prepared[at].r2=vec4<u32>(q6,q7,q8,q9);
}
@compute @workgroup_size(8,4) fn gi_receiver_pass(@builtin(global_invocation_id) grid:vec3<u32>) {
    let id=grid.xy+primary_region.xy;
    let size=textureDimensions(primary_hit);
    if any(id>=size) || any(id>=primary_region.zw) {return;}
    if textureLoad(primary_tiles,id/PRIMARY_TILE,0).x==0u {return;}
    let record=textureLoad(primary_hit,vec2<i32>(id),0);
    if record.x==0u {return;}
    let hit=primary_decode(record);
    let a=primary_input(hit.batch,hit.ia,hit.placement);
    let b=primary_input(hit.batch,hit.ib,hit.placement);
    let c=primary_input(hit.batch,hit.ic,hit.placement);
    let w=vec3<f32>(1.0-hit.bary.x-hit.bary.y,hit.bary);
    var input:SurfaceInput;
    input.receiver.world=world_position(receiver_point(a.position,b.position,c.position,w),hit.placement).xyz;
    input.receiver.geometric=primary_geometric(a.position,b.position,c.position,hit.placement);
    input.normal=vertex_output(a).normal*w.x+vertex_output(b).normal*w.y+vertex_output(c).normal*w.z;
    gi_prepare(input);
    gi_rx_store_at(id.y*size.x+id.x);
}

// ---- final gather (RENDERLAB_FINAL_GATHER, src/gi_receiver_pass.rs) ----
// Pixels never read the lighting cache's cells: the resolve kernel contains no cell lookup and
// no shading program (the cache is read only at ray hits, inside the hit's own shading). Three
// kernels:
// - fg_trace: screen probes, FG_SLOTS per FG_TILE x FG_TILE tile on the surfaces under covered
//   pixels the frame picks (slot 1 on a second plane where the tile holds one), each trace
//   FG_RAYS rays over their hemisphere (hardware rays or the software trace: the module's
//   rl_cast). A hit takes its cache cell's outgoing radiance (fg_radiance); cells reach the
//   screen only through a hemisphere integral. Misses see the sky.
// - fg_filter: each tile probe's filtered copy, the probes around it on its plane weighted by
//   their frames of history (3x3 tiles, 5x5 while its own history is short, as after a cut):
//   neighbours' samples count as its own, so the interpolated light shows no probe pattern.
//   History accumulates on the unfiltered probes.
// - fg_adapt: adaptive probes (as in Lumen's screen probe gather), one per FG_BLOCK block,
//   placed only where a covered pixel of the block has no tile probe on its plane, so edges,
//   thin parts and third planes get light of their own; blocks without such a pixel exit
//   after the test.
// - fg_resolve: each pixel takes the filtered tile probes around it on its own plane (position, plane
//   distance and normal weights), else the adaptive probes around it, else the nearest probe
//   on its plane within FG_SEARCH tiles, else the facing probe of its own tile or block.
// Probes keep radiance as L2 SH, accumulated over frames while their new point stays on their
// plane. The resolve writes the record shading reads (sky slots absolute: gi_api.wgsl
// gi_rx_absolute).
const FG_TILE:u32=16u;
const FG_SLOTS:u32=2u;
const FG_BLOCK:u32=8u;
const FG_RAYS:u32=16u;
const FG_HISTORY:f32=16.0;
// Half-rate tracing (fg_trace and fg_adapt) for probes with at least FG_HALF_MIN frames of history.
const FG_HALF_RATE:bool=true;
const FG_HALF_MIN:f32=4.0;
// A probe ray's cone: its share of the hemisphere (2 pi / FG_RAYS sr) as an angle, so its
// width at distance t is FG_CONE * t (the cache answers hits whose cone covers a cell).
const FG_CONE:f32=0.6266570686577501;
// The sky a probe ray misses into: the map's own radiance in that direction (its base level).
// The probe's rays and history average it over the hemisphere. A lookup prefiltered to the ray's
// cone (GGX alpha 0.098) blurred the bright haze below the horizon into the directions above it:
// with every ray forced to miss, probes integrated 10.3% more than the open sky's irradiance
// (gi_open_irradiance) on Glint's floor, the sky part of the gather's overshoot in sky-lit shadow.
// At alpha 0 they integrate it within 0.01%. The whole hemisphere's blur, before that, had mixed
// the horizon into the zenith and greyed sky-lit shadow.
const FG_SKY_ALPHA:f32=0.0;
const FG_SEARCH:i32=2;
// A probe's plane reaches this fraction of its footprint along its normal (the Gaussian width).
const FG_PLANE:f32=0.5;
// The incoming radiance as the lighting cache splits it, so shading reads a probe as it reads a
// cell (the sky's visibility for specular, es_prepare, and the bounce lobe of glossy surfaces,
// gi_rx_bounce_radiance): floats 0-26 the sky the probe's rays missed into (L2), 27 a pad, 28-39
// everything they met and the emitters (L1). Then the probe point (w: frames of history, 0 when
// invalid) and normal (w: the tile footprint in metres). Probes live after the pixels' records
// in gi_prepared, four records a probe: the tile probes, one adaptive probe per FG_BLOCK block,
// then the tile probes' filtered copies (src/gi_receiver_pass.rs sizes it).
const FG_SH_VECS:u32=10u;
const FG_BOUNCE:u32=28u;
struct FgProbe { sh:array<vec4<f32>,10>, point:vec4<f32>, normal:vec4<f32> };
fn fg_at(index:u32,size:vec2<u32>)->u32 { return size.x*size.y+4u*index; }
fn fg_adaptive(block:vec2<u32>,size:vec2<u32>)->u32 {
    let tiles=(size+vec2<u32>(FG_TILE-1u))/FG_TILE;
    let blocks=(size+vec2<u32>(FG_BLOCK-1u))/FG_BLOCK;
    return tiles.x*tiles.y*FG_SLOTS+block.y*blocks.x+block.x;
}
fn fg_filtered(index:u32,size:vec2<u32>)->u32 {
    let tiles=(size+vec2<u32>(FG_TILE-1u))/FG_TILE;
    let blocks=(size+vec2<u32>(FG_BLOCK-1u))/FG_BLOCK;
    return tiles.x*tiles.y*FG_SLOTS+blocks.x*blocks.y+index;
}
fn fg_load(index:u32,size:vec2<u32>)->FgProbe {
    let at=fg_at(index,size);
    let a=gi_prepared[at];let b=gi_prepared[at+1u];let c=gi_prepared[at+2u];let e=gi_prepared[at+3u];
    return FgProbe(array<vec4<f32>,10>(bitcast<vec4<f32>>(a.r0),bitcast<vec4<f32>>(a.r1),bitcast<vec4<f32>>(a.r2),
        bitcast<vec4<f32>>(b.r0),bitcast<vec4<f32>>(b.r1),bitcast<vec4<f32>>(b.r2),bitcast<vec4<f32>>(c.r0),
        bitcast<vec4<f32>>(c.r1),bitcast<vec4<f32>>(c.r2),bitcast<vec4<f32>>(e.r0)),
        bitcast<vec4<f32>>(e.r1),bitcast<vec4<f32>>(e.r2));
}
// A probe's point and normal alone (the last two words of its fourth record): the plane tests
// read these, and the 160 bytes of SH load only once a weight is known (fg_add).
struct FgPlane { point:vec4<f32>, normal:vec4<f32> };
// The weighted SH sum fg_add adds to (private, not a pointer argument: naga's SPIR-V writer
// fails on a pointer to a local struct's member).
var<private> fg_sum:array<vec4<f32>,10>;
fn fg_clear() { for(var v=0u;v<FG_SH_VECS;v+=1u) {fg_sum[v]=vec4<f32>(0.0);} }
fn fg_plane(index:u32,size:vec2<u32>)->FgPlane {
    let c=gi_prepared[fg_at(index,size)+3u];
    return FgPlane(bitcast<vec4<f32>>(c.r1),bitcast<vec4<f32>>(c.r2));
}
fn fg_add(index:u32,size:vec2<u32>,w:f32) {
    let at=fg_at(index,size);
    let a=gi_prepared[at];let b=gi_prepared[at+1u];let c=gi_prepared[at+2u];let e=gi_prepared[at+3u].r0;
    fg_sum[0]+=w*bitcast<vec4<f32>>(a.r0);fg_sum[1]+=w*bitcast<vec4<f32>>(a.r1);fg_sum[2]+=w*bitcast<vec4<f32>>(a.r2);
    fg_sum[3]+=w*bitcast<vec4<f32>>(b.r0);fg_sum[4]+=w*bitcast<vec4<f32>>(b.r1);fg_sum[5]+=w*bitcast<vec4<f32>>(b.r2);
    fg_sum[6]+=w*bitcast<vec4<f32>>(c.r0);fg_sum[7]+=w*bitcast<vec4<f32>>(c.r1);fg_sum[8]+=w*bitcast<vec4<f32>>(c.r2);
    fg_sum[9]+=w*bitcast<vec4<f32>>(e);
}
fn fg_store(index:u32,size:vec2<u32>,p:FgProbe) {
    let at=fg_at(index,size);
    gi_prepared[at]=GiPrepared(bitcast<vec4<u32>>(p.sh[0]),bitcast<vec4<u32>>(p.sh[1]),bitcast<vec4<u32>>(p.sh[2]));
    gi_prepared[at+1u]=GiPrepared(bitcast<vec4<u32>>(p.sh[3]),bitcast<vec4<u32>>(p.sh[4]),bitcast<vec4<u32>>(p.sh[5]));
    gi_prepared[at+2u]=GiPrepared(bitcast<vec4<u32>>(p.sh[6]),bitcast<vec4<u32>>(p.sh[7]),bitcast<vec4<u32>>(p.sh[8]));
    gi_prepared[at+3u]=GiPrepared(bitcast<vec4<u32>>(p.sh[9]),bitcast<vec4<u32>>(p.point),bitcast<vec4<u32>>(p.normal));
}
fn fg_invalidate(index:u32,size:vec2<u32>) {
    let at=fg_at(index,size)+3u;
    gi_prepared[at]=GiPrepared(gi_prepared[at].r0,vec4<u32>(0u),vec4<u32>(0u));
}
// Per lane: its ray's terms in the probe's layout (sky L2 at 0-26 for a miss, bounce L1 at
// FG_BOUNCE for a hit, and 1 at FG_ANSWERED, the layout's pad, when the ray counts), and its
// emitter sample's bounce L1 terms (fg_nee).
var<workgroup> fg_lane_sh:array<array<f32,40>,32>;
var<workgroup> fg_nee_sh:array<array<f32,12>,32>;
const FG_ANSWERED:u32=27u;

fn fg_hash(x:u32)->u32 {
    var h=x*0x9e3779b1u;h^=h>>16u;h*=0x85ebca6bu;h^=h>>13u;h*=0xc2b2ae35u;h^=h>>16u;return h;
}
// A covered pixel's receiver for the gather: point, viewed geometric normal, shading normal,
// trace epsilon and placement + 1 (0: none).
struct FgSurface { world:vec3<f32>, normal:vec3<f32>, shading:vec3<f32>, epsilon:f32, placement:u32 };
fn fg_surface(id:vec2<u32>,size:vec2<u32>)->FgSurface { return fg_surface_at(id,size,true); }
// fg_surface, without the shading normal (left as the geometric one) unless `shading`.
fn fg_surface_at(id:vec2<u32>,size:vec2<u32>,shading:bool)->FgSurface {
    var s:FgSurface;
    if any(id>=size) || any(id>=primary_region.zw) || any(id<primary_region.xy) {return s;}
    if textureLoad(primary_tiles,id/PRIMARY_TILE,0).x==0u {return s;}
    let record=textureLoad(primary_hit,vec2<i32>(id),0);
    if record.x==0u {return s;}
    let hit=primary_decode(record);
    let a=primary_input(hit.batch,hit.ia,hit.placement);
    let b=primary_input(hit.batch,hit.ib,hit.placement);
    let c=primary_input(hit.batch,hit.ic,hit.placement);
    let w=vec3<f32>(1.0-hit.bary.x-hit.bary.y,hit.bary);
    s.world=world_position(receiver_point(a.position,b.position,c.position,w),hit.placement).xyz;
    let g=unit_normal(primary_geometric(a.position,b.position,c.position,hit.placement));
    s.normal=select(-g,g,dot(g,lighting.eye-s.world)>=0.0);
    s.shading=s.normal;
    if shading {
        let n=vertex_output(a).normal*w.x+vertex_output(b).normal*w.y+vertex_output(c).normal*w.z;
        s.shading=select(s.normal,unit_normal(n),dot(n,n)>1e-12);
        if dot(s.shading,s.normal)<0.0 {s.shading=-s.shading;}
    }
    s.epsilon=primary_epsilon(array<vec3<f32>,3>(a.position,b.position,c.position),hit.placement);
    s.placement=record.x;
    return s;
}
// The world size of FG_TILE pixels at a point: the plane tests' scale.
fn fg_footprint(pixel:vec2<u32>,size:vec2<u32>,world:vec3<f32>)->f32 {
    let ray=primary_ray(pixel,size);
    let pixel_angle=length(ray.dx_far-ray.dx_near)/max(length(ray.far-ray.near),1e-30);
    return f32(FG_TILE)*pixel_angle*length(world-lighting.eye);
}
// How far a point and normal lie on a probe's plane (Gaussian in plane distance over
// FG_PLANE of its footprint, normals as cos^4): 1 on it, 0 off it or for an invalid probe.
fn fg_on_plane(p:FgPlane,world:vec3<f32>,n:vec3<f32>)->f32 {
    if p.point.w<=0.0 {return 0.0;}
    let plane=abs(dot(p.normal.xyz,world-p.point.xyz))/max(FG_PLANE*p.normal.w,1e-6);
    let facing=max(dot(p.normal.xyz,n),0.0);
    return exp(-plane*plane)*facing*facing*facing*facing;
}
fn fg_basis(n:vec3<f32>)->mat3x3<f32> {
    let s=select(-1.0,1.0,n.z>=0.0);
    let a=-1.0/(s+n.z);let b=n.x*n.y*a;
    return mat3x3<f32>(vec3<f32>(1.0+s*n.x*n.x*a,s*b,-s*n.x),vec3<f32>(b,s+n.y*n.y*a,-n.y),n);
}
// Uniform hemisphere direction k of `count`, turned by r.
// The height z (uniform in solid angle) is stratified by k over `count` and the azimuth follows a
// golden-ratio sequence, both jittered and turned by r, so every count covers the hemisphere
// evenly. A side x side grid left two cells near the zenith empty at 14 rays: the irradiance's
// cosine term then read low (gicut's furnace: a slab under an open sky 8% dark).
fn fg_direction(n:vec3<f32>,k:u32,count:u32,r:u32)->vec3<f32> {
    let jitter=vec2<f32>(f32(r&0xffffu),f32(r>>16u))*(1.0/65536.0);
    let z=(f32(k)+jitter.x)/f32(count);
    let rr=sqrt(max(1.0-z*z,0.0));
    let phi=6.283185307179586*fract(f32(k)*0.6180339887498949+jitter.y);
    return fg_basis(n)*vec3<f32>(rr*cos(phi),rr*sin(phi),z);
}
// Incoming radiance (unexposed) along d from `origin`. A hit reads its lighting-cache cell's
// outgoing radiance, the only hit path: cell shading ran the hit's own program there, so the
// value carries that program's emission and albedo (Glint's white surfaces glow; Outpost's
// terrain and Ara's materials have their own programs) with the cell's indirect and direct
// light. The gather's module holds only the standard program, so shading the hit itself lost
// all of that: a probe hit on Glint's white surfaces came out without their glow, a tenth dark
// in sky-lit shadow. rl_cache_cell picks the cell (reflection_cache_hits.wgsl); a hit no level
// answers (or any hit while RENDERLAB_RL_CACHE_HITS=off leaves the stub in) sends nothing.
fn fg_radiance(origin:vec3<f32>,d:vec3<f32>,epsilon:f32)->vec3<f32> {
    let traced=rl_cast(origin,d,epsilon);
    fg_hit=traced.record.x!=0u;
    fg_unanswered=false;
    fg_no_value=false;
    if traced.record.x==0u {return rl_sky(d,FG_SKY_ALPHA)/max(lighting.exposure,1e-30);}
    let cell=rl_cache_cell(traced.record,origin,d,traced.t,FG_CONE*traced.t);
    fg_touch(rl_cache_slots.x);
    fg_touch(rl_cache_slots.y);
    fg_no_value=cell.w==0.0;
    fg_unanswered=fg_no_value;
    // A ray that meets an emitting draw keeps its cell's reflected light and its emission weighted
    // against the probe's emitter samples (fg_nee, multiple importance sampling).
    let hit=primary_decode(traced.record);
    let emission=gi_triangle_emission(hit.batch,hit.placement,traced.record.y);
    if all(emission<=vec3<f32>(0.0)) {return rl_finite(cell.rgb);}
    // The triangle's emission is known exactly from the list, so the hit counts even when its
    // cell has no value (thin emitting strips' fine cells are often orphans that never get one);
    // only its reflected light, small on Glint's dark emitter bases, is then missing.
    fg_unanswered=false;
    let light=fg_light_pdf(fg_luminance(emission),traced.t,abs(dot(unit_normal(rl_cache_geometric),d)));
    let reflected=select(vec3<f32>(0.0),max(cell.rgb-emission,vec3<f32>(0.0)),cell.w>0.0);
    return rl_finite(reflected+emission*fg_mis(FG_HEMISPHERE_PDF*f32(FG_HEMI),light*f32(FG_NEE)));
}
// Probe rays are uniform over the hemisphere: their density in solid angle.
const FG_HEMISPHERE_PDF:f32=0.15915494309189535;
fn fg_luminance(c:vec3<f32>)->f32 { return dot(c,vec3<f32>(0.2126,0.7152,0.0722)); }
// The density (per steradian) with which fg_nee's emitter sampling picks a direction meeting an
// emitter of luminance `luminance`, `t` away, whose surface makes cosine `cosine` with the ray:
// the list picks a triangle in proportion to its power (luminance pi area over the total) and a
// point of it uniformly, so the area cancels. 0 when the list is empty.
fn fg_light_pdf(luminance:f32,t:f32,cosine:f32)->f32 {
    let n=gi_word(GI_H_EMITTER_COUNT);
    if n==0u {return 0.0;}
    let total=bitcast<f32>(gi_word(gi_emitter_cdf_at()+n-1u));
    if !(total>0.0) {return 0.0;}
    return luminance*3.141592653589793*t*t/(total*max(cosine,1e-4));
}
// The power heuristic's weight for a sample from the strategy of (count times density) `own`
// against the other's `other` (Veach 1997).
fn fg_mis(own:f32,other:f32)->f32 {
    let a=own*own;
    return a/max(a+other*other,1e-30);
}
// Whether the last fg_radiance ray met geometry (bounce) or missed into the sky, and whether its
// hit found no cell value (rl_cache_missed names the cell).
var<private> fg_hit:bool;
var<private> fg_unanswered:bool;
// Whether its hit's cell had no value (it is then requested, fg_request), answered or not.
var<private> fg_no_value:bool;
// The lighting cache's external request list (gi_cache.wgsl gi_external, GiState::
// external_requests): a probe hit without a value asks for its cell, which the cache's next frame
// requests a band below pixels'. The gather's own group 2 binds it (src/gi_receiver_pass.rs; the
// probes sample no material). Word 0 the count, 1 the overflow, then four words an entry. One
// request a probe a frame (fg_asked, one flag per probe of the workgroup): many of a probe's rays
// meet the same cell.
@group(2) @binding(30) var<storage,read_write> fg_requests:array<atomic<u32>>;
const FG_REQUESTS_MAX:u32=16384u;
// After the entries, one bit per cache slot: the cells this frame's probe hits read, or found
// without a value at their cone level. The cache stamps them seen, as its own readers' cells, so
// they are not taken from a full bucket while the gather still reads them, and schedules those
// still without a value (gi_cache.wgsl GI_EXTERNAL_SEEN).
const FG_SEEN:u32=4u+4u*FG_REQUESTS_MAX;
fn fg_touch(slot:u32) {
    if slot==0xffffffffu { return; }
    let w=FG_SEEN+(slot>>5u);
    if w<arrayLength(&fg_requests) { atomicOr(&fg_requests[w],1u<<(slot&31u)); }
}
// Stable probes. A tile probe whose last FG_STABLE traced updates saw no change traces one frame
// in FG_STABLE_RATE instead of every other frame (half rate). Each update gets the same rays, so
// a stable probe's noise is unchanged; only a change it has not traced yet waits longer. It
// returns to the half rate as soon as it or a tile probe the filter blends into it (the 3 x 3
// tiles around it: fg_filter reaches one tile for probes past FG_FILTER_SHORT frames of history,
// as stable ones are) sees a change, and every probe does on a cut, a camera jump or a lighting edit (word 0, set
// from the cache's planner: src/gi_receiver_pass.rs). Per probe (tile probes, then one per
// block, as fg_at numbers them) two words from 2: its traced updates in a row without a change
// (bits 0-7, saturating) with the frame of its last change (bits 8-31), and the f32 bits of its
// usual change (fg_note_change). Word 1 is the gather's frame count (fg_frame).
const FG_STABLE:u32=4u;
const FG_STABLE_RATE:u32=4u;
@group(2) @binding(31) var<storage,read_write> fg_state:array<u32>;
fn fg_state_at(index:u32)->u32 { return 2u+2u*index; }
// The gather's own frame count, written by the CPU every frame (word 1 of fg_state): its tile
// picks, ray directions and half-rate checkerboard turn with it. They turned with
// primary_frame.sided.y, which is TR1's direction rotation (8 * (frame % 32), 0 with TR1 off):
// its multiples of 8 never changed the checkerboard's parity, so the same half of the tile
// probes was kept on every frame and stopped updating once it had history on its plane, and
// with TR1 off every frame traced the same directions.
fn fg_frame()->u32 { return fg_state[1]; }
// Whether a probe's last change was this frame or the one before (24-bit frame stamps).
fn fg_changed_recently(word:u32,frame:u32)->bool { return ((frame-(word>>8u))&0xffffffu)<=1u; }
var<workgroup> fg_moved:atomic<u32>;
var<workgroup> fg_asked:array<atomic<u32>,2>;
// The workgroup's probe rays and emitter shadow rays, added to the external list's spare words 2
// and 3, which the GI log prints as fg_rays and fg_shadow_rays.
var<workgroup> fg_rays:array<atomic<u32>,2>;
fn fg_count_rays(lane:u32) {
    if lane==0u {
        atomicAdd(&fg_requests[2u],atomicLoad(&fg_rays[0]));
        atomicAdd(&fg_requests[3u],atomicLoad(&fg_rays[1]));
    }
}
fn fg_request(cell:vec4<u32>) {
    let i=atomicAdd(&fg_requests[0],1u);
    if i>=min(FG_REQUESTS_MAX,(arrayLength(&fg_requests)-4u)/4u) {atomicAdd(&fg_requests[1],1u);return;}
    let at=4u+4u*i;
    atomicStore(&fg_requests[at],cell.x);
    atomicStore(&fg_requests[at+1u],cell.y);
    atomicStore(&fg_requests[at+2u],cell.z);
    atomicStore(&fg_requests[at+3u],cell.w);
}
// Traces lane `ray` of a probe at s and stores its SH terms in fg_lane_sh[lane]: a miss's as the
// sky's L2, a hit's as the bounce's L1 (zero when the probe has no surface).
fn fg_lane(lane:u32,ray:u32,s:FgSurface,pixel:vec2<u32>,r:u32) {
    var radiance=vec3<f32>(0.0);
    var d=vec3<f32>(0.0,0.0,1.0);
    fg_hit=false;
    var answered=false;
    for(var i=0u;i<40u;i+=1u) {fg_lane_sh[lane][i]=0.0;}
    for(var i=0u;i<12u;i+=1u) {fg_nee_sh[lane][i]=0.0;}
    if s.placement!=0u {
        if ray<FG_NEE {
            // An emitter sample takes this lane's one ray (fg_nee).
            fg_nee(lane,ray,s,r);
        } else {
            d=fg_direction(s.normal,ray-FG_NEE,FG_HEMI,r);
            radiance=fg_radiance(fma(s.normal,vec3<f32>(s.epsilon),s.world),d,s.epsilon);
            atomicAdd(&fg_rays[0],1u);
            if fg_hit && fg_no_value && rl_cache_missed.x!=0xffffffffu && atomicExchange(&fg_asked[lane/FG_RAYS],1u)==0u {fg_request(rl_cache_missed);}
            // A hit no cache level answers yet is left out of the probe's average (fg_accumulate
            // counts the answered rays in the pad float FG_ANSWERED): counted as black, it
            // darkened probes after a cut until the cells it asked for were shaded.
            answered=!(fg_hit && fg_unanswered);
        }
    }
    if !answered {return;}
    gi_sh_basis(d);
    let sky=select(radiance,vec3<f32>(0.0),fg_hit);
    let bounce=select(vec3<f32>(0.0),radiance,fg_hit);
    fg_lane_sh[lane][FG_ANSWERED]=1.0;
    for(var i=0u;i<9u;i+=1u) {
        fg_lane_sh[lane][i*3u]=sky.x*gi_y[i];fg_lane_sh[lane][i*3u+1u]=sky.y*gi_y[i];fg_lane_sh[lane][i*3u+2u]=sky.z*gi_y[i];
    }
    for(var k=0u;k<4u;k+=1u) {
        fg_lane_sh[lane][FG_BOUNCE+k*3u]=bounce.x*gi_y[k];fg_lane_sh[lane][FG_BOUNCE+k*3u+1u]=bounce.y*gi_y[k];fg_lane_sh[lane][FG_BOUNCE+k*3u+2u]=bounce.z*gi_y[k];
    }
}
// Emitters as lights (next-event estimation from the lighting cache's emitter list,
// gi_emitters.wgsl, the list the cache's own gathers sample), combined with the probe's own rays
// by multiple importance sampling: lanes 0 .. FG_NEE of a probe each pick one emitter triangle in
// proportion to its power (gi_emitter_pick, the list's power CDF) and a point of it, and where
// one ray to it is unblocked add its radiance in that direction weighted by the power heuristic
// against the probe's FG_HEMI hemisphere rays; a hemisphere ray that meets an emitter takes the
// complementary weight (fg_radiance). Small bright emitters (lamps, Glint's orange rims) then
// come mostly from the samples, and large dim ones (Glint's glowing white surfaces, nearly all
// the list's power) mostly from the rays: picked by power alone, a probe's nearby white floor was
// almost never sampled, and its rare picks' large weights did not survive TR1. Divided by
// fg_accumulate's per-ray scale, so the sums come out exact. The samples take FG_NEE of a probe's
// FG_RAYS lanes, each tracing its one ray alongside the FG_HEMI hemisphere rays: traced after
// them on the same lanes, they doubled the warp's traversal rounds (fg_trace +0.53 ms without
// ray hardware on Glint).
const FG_NEE:u32=2u;
const FG_HEMI:u32=FG_RAYS-FG_NEE;
fn fg_nee(lane:u32,ray:u32,s:FgSurface,r:u32) {
    let h=fg_hash(r^((ray+1u)*0x9e3779b9u));
    let pick=gi_emitter_pick(f32(h>>8u)*(1.0/16777216.0));
    if pick.probability<=0.0 {return;}
    let w=gi_word(GI_H_EMITTERS)+pick.index*GI_EMITTER_WORDS;
    let e1=gi_emitter_vec(w+3u);
    let e2=gi_emitter_vec(w+6u);
    let g=fg_hash(h);
    var uv=vec2<f32>(f32(g&0xffffu),f32(g>>16u))*(1.0/65536.0);
    if uv.x+uv.y>1.0 {uv=vec2<f32>(1.0)-uv;}
    let origin=fma(s.normal,vec3<f32>(s.epsilon),s.world);
    let to=gi_emitter_vec(w)+e1*uv.x+e2*uv.y-origin;
    let dist=length(to);
    if dist<=1e-6 {return;}
    let d=to/dist;
    if dot(d,s.normal)<=0.0 {return;}
    let side=cross(e1,e2);
    let area=0.5*length(side);
    let cosine=abs(dot(side,d))*0.5/max(area,1e-30);
    if area<=0.0 || cosine<=1e-4 {return;}
    let density=pick.probability*dist*dist/(area*cosine);
    atomicAdd(&fg_rays[1],1u);
    if rl_blocked(origin,d,s.epsilon,dist*0.999) {return;}
    let weight=fg_mis(density*f32(FG_NEE),FG_HEMISPHERE_PDF*f32(FG_HEMI));
    let light=gi_emitter_vec(w+9u)*(weight*FG_HEMISPHERE_PDF*f32(FG_HEMI)/(density*f32(FG_NEE)));
    gi_sh_basis(d);
    for(var k=0u;k<4u;k+=1u) {
        fg_nee_sh[lane][k*3u]+=light.x*gi_y[k];
        fg_nee_sh[lane][k*3u+1u]+=light.y*gi_y[k];
        fg_nee_sh[lane][k*3u+2u]+=light.z*gi_y[k];
    }
}
// Sums each probe's FG_RAYS lanes into its first lane's row, each lane summing its own coefficients
// (in the lanes' order, so the sums are a serial loop's), for fg_accumulate. One lane summing all 16
// rows of 52 alone, its probe's other lanes idle, cost 0.45 ms on Glint without ray hardware.
fn fg_reduce(lane:u32) {
    let first=(lane/FG_RAYS)*FG_RAYS;
    for(var i=lane%FG_RAYS;i<52u;i+=FG_RAYS) {
        var sum=0.0;
        if i<40u {
            for(var l=0u;l<FG_RAYS;l+=1u) {sum+=fg_lane_sh[first+l][i];}
            fg_lane_sh[first][i]=sum;
        } else {
            for(var l=0u;l<FG_RAYS;l+=1u) {sum+=fg_nee_sh[first+l][i-40u];}
            fg_nee_sh[first][i-40u]=sum;
        }
    }
}
// The luminance of a probe's irradiance at n (its sky L2 and bounce L1 through the clamped cosine).
fn fg_irradiance(sh:array<vec4<f32>,10>,n:vec3<f32>)->f32 {
    gi_sh_basis(n);
    var e=vec3<f32>(0.0);
    for(var i=0u;i<9u;i+=1u) {
        let a=i*3u;
        e+=gi_band_weight(i)*gi_y[i]*vec3<f32>(sh[a/4u][a%4u],sh[(a+1u)/4u][(a+1u)%4u],sh[(a+2u)/4u][(a+2u)%4u]);
    }
    for(var i=0u;i<4u;i+=1u) {
        let b=FG_BOUNCE+i*3u;
        e+=gi_band_weight(i)*gi_y[i]*vec3<f32>(sh[b/4u][b%4u],sh[(b+1u)/4u][(b+1u)%4u],sh[(b+2u)/4u][(b+2u)%4u]);
    }
    return dot(max(e,vec3<f32>(0.0)),vec3<f32>(0.2126,0.7152,0.0722));
}
// A traced update's change against the probe's history (fg_state): the relative difference of
// their irradiance at the probe's normal. A change is one above FG_CHANGE times the probe's usual
// change (a running mean of the updates that were not changes: a fresh update of FG_HEMI rays
// differs from the history by its noise, which varies from probe to probe) and above
// FG_CHANGE_MIN. Probes without FG_HALF_MIN frames of history only learn their usual change.
// On Glint's settled views 2.5 times the usual change flagged 4-6% of updates, nearly all noise,
// which kept three probes in four at the half rate through their neighbours; 4 times flags 1%.
// Returns whether this update is a change.
const FG_CHANGE:f32=4.0;
const FG_CHANGE_MIN:f32=0.15;
fn fg_note_change(index:u32,fresh:array<vec4<f32>,10>,old:FgProbe,count:f32,n:vec3<f32>)->bool {
    let at=fg_state_at(index);
    if at+1u>=arrayLength(&fg_state) {return false;}
    let word=fg_state[at];
    var usual=bitcast<f32>(fg_state[at+1u]);
    if count<=1.0 {fg_state[at]=word&0xffffff00u;fg_state[at+1u]=bitcast<u32>(0.25);return false;}
    let e_new=fg_irradiance(fresh,n);
    let e_old=fg_irradiance(old.sh,n);
    let r=abs(e_new-e_old)/max(max(e_new,e_old),1e-30);
    if old.point.w>=FG_HALF_MIN && r>max(FG_CHANGE*usual,FG_CHANGE_MIN) {
        fg_state[at]=(fg_frame()&0xffffffu)<<8u;
        return true;
    }
    usual=mix(usual,r,0.125);
    fg_state[at]=(word&0xffffff00u)|min((word&0xffu)+1u,255u);
    fg_state[at+1u]=bitcast<u32>(usual);
    return false;
}
// The history a probe keeps through its own change (fg_note_change; a lighting edit's word 0 only
// sets the rate: it is on through whole relight spreads, where restarting every probe would add
// noise everywhere): FG_RESTART updates, so the new light takes over in a few updates instead of
// blending in at 1/FG_HISTORY an update (a light switched on under settled probes reached 0.75
// of its light 48 frames later), and the probe does not turn stable while far from it.
const FG_RESTART:f32=4.0;
// Folds the lanes from `first` (summed into that row by fg_reduce) into probe `index`, now at s:
// history while the new point stays on the old probe's plane.
fn fg_accumulate(index:u32,size:vec2<u32>,first:u32,s:FgSurface,pixel:vec2<u32>) {
    var sh:array<f32,40>;
    var nee:array<f32,12>;
    // fg_reduce left each sum in the probe's first row.
    for(var i=0u;i<40u;i+=1u) {sh[i]=fg_lane_sh[first][i];}
    for(var i=0u;i<12u;i+=1u) {nee[i]=fg_nee_sh[first][i];}
    // Uniform hemisphere: pdf 1/(2 pi), over the rays with an answer (the others stand for their
    // average). The emitter samples were divided by the full ray count's scale (fg_nee).
    let scale=6.283185307179586/max(sh[FG_ANSWERED],1.0);
    sh[FG_ANSWERED]=0.0;
    for(var i=0u;i<40u;i+=1u) {sh[i]*=scale;}
    for(var i=0u;i<12u;i+=1u) {sh[FG_BOUNCE+i]+=nee[i]*(6.283185307179586/f32(FG_HEMI));}
    let footprint=fg_footprint(pixel,size,s.world);
    let old=fg_load(index,size);
    var count=1.0;
    if old.point.w>0.0 && dot(old.normal.xyz,s.normal)>0.9 && abs(dot(old.normal.xyz,s.world-old.point.xyz))<0.5*FG_PLANE*footprint {
        count=min(old.point.w+1.0,FG_HISTORY);
    }
    var fresh:array<vec4<f32>,10>;
    for(var v=0u;v<FG_SH_VECS;v+=1u) {fresh[v]=vec4<f32>(sh[v*4u],sh[v*4u+1u],sh[v*4u+2u],sh[v*4u+3u]);}
    if fg_note_change(index,fresh,old,count,s.normal) {count=min(count,FG_RESTART);}
    let blend=1.0/count;
    var out:FgProbe;
    for(var v=0u;v<FG_SH_VECS;v+=1u) {out.sh[v]=select(fresh[v],mix(old.sh[v],fresh[v],blend),count>1.0);}
    out.point=vec4<f32>(s.world,count);
    out.normal=vec4<f32>(s.normal,footprint);
    fg_store(index,size,out);
}
@compute @workgroup_size(32) fn fg_trace(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let size=textureDimensions(primary_hit);
    let tiles=(size+vec2<u32>(FG_TILE-1u))/FG_TILE;
    let tile=group.y*tiles.x+group.x;
    let frame=fg_frame();
    let slot=lane/FG_RAYS;
    if lane<2u {atomicStore(&fg_asked[lane],0u);atomicStore(&fg_rays[lane],0u);}
    if lane==0u {atomicStore(&fg_moved,0u);}
    workgroupBarrier();
    // A change at a tile probe the filter blends in (3 x 3 tiles, both slots) this frame or the
    // last keeps this tile's probes at the half rate: the filter would carry it here.
    if lane<9u {
        let t=vec2<i32>(group.xy)+vec2<i32>(i32(lane%3u)-1,i32(lane/3u)-1);
        if all(t>=vec2<i32>(0)) && all(t<vec2<i32>(tiles)) {
            let near=u32(t.y)*tiles.x+u32(t.x);
            for(var k=0u;k<FG_SLOTS;k+=1u) {
                let at=fg_state_at(near*FG_SLOTS+k);
                if at<arrayLength(&fg_state) && fg_changed_recently(fg_state[at],frame) {atomicOr(&fg_moved,1u);}
            }
        }
    }
    workgroupBarrier();
    // Slot 0: the first covered pixel of four the frame picks in the tile. Slot 1: the first of
    // eight more off slot 0's plane (every lane of a slot computes the same).
    var first:FgSurface;
    var first_pixel=vec2<u32>(0u);
    for(var k=0u;k<4u;k+=1u) {
        let h=fg_hash(tile*16u+k+frame*0x632be5abu);
        let q=group.xy*FG_TILE+vec2<u32>(h%FG_TILE,(h>>8u)%FG_TILE);
        first=fg_surface_at(q,size,false);
        if first.placement!=0u {first_pixel=q;break;}
    }
    var s=first;
    var pixel=first_pixel;
    if slot==1u {
        s=FgSurface();
        if first.placement!=0u {
            var probe:FgPlane;
            probe.point=vec4<f32>(first.world,1.0);
            probe.normal=vec4<f32>(first.normal,fg_footprint(first_pixel,size,first.world));
            for(var k=4u;k<12u;k+=1u) {
                let h=fg_hash(tile*16u+k+frame*0x632be5abu);
                let q=group.xy*FG_TILE+vec2<u32>(h%FG_TILE,(h>>8u)%FG_TILE);
                let c=fg_surface_at(q,size,false);
                if c.placement!=0u && fg_on_plane(probe,c.world,c.normal)<0.37 {s=c;pixel=q;break;}
            }
        }
    }
    let index=tile*FG_SLOTS+slot;
    // Half rate: on alternate frames, a checkerboard of tiles keeps a probe that has history and
    // still lies on the plane under its tile (fg_accumulate's test), instead of tracing it. A
    // stable probe (fg_state) traces one frame in FG_STABLE_RATE.
    let old=fg_plane(index,size);
    let at=fg_state_at(index);
    let stable=fg_state[0]==0u && at<arrayLength(&fg_state) && (fg_state[at]&0xffu)>=FG_STABLE && atomicLoad(&fg_moved)==0u;
    let turn=select((group.x+group.y+frame)&1u,(group.x+2u*group.y+frame)%FG_STABLE_RATE,stable);
    let keep=FG_HALF_RATE && turn!=0u && s.placement!=0u && old.point.w>=FG_HALF_MIN
        && dot(old.normal.xyz,s.normal)>0.9 && abs(dot(old.normal.xyz,s.world-old.point.xyz))<0.5*FG_PLANE*old.normal.w;
    var traced=s;
    if keep {traced.placement=0u;}
    fg_lane(lane,lane%FG_RAYS,traced,pixel,fg_hash(index^(frame*0x27d4eb2fu)));
    workgroupBarrier();
    fg_reduce(lane);
    workgroupBarrier();
    fg_count_rays(lane);
    if lane%FG_RAYS!=0u || any(group.xy>=tiles) || keep {return;}
    if s.placement==0u {fg_invalidate(index,size);return;}
    fg_accumulate(index,size,slot*FG_RAYS,s,pixel);
}
// The tile probes around a pixel on its plane, bilinear by tile position: weighted SH and the
// weights' sum.
// Returns the weights' sum and, when `sh`, adds the weighted SH to fg_sum (the adaptive test
// needs the sum alone).
fn fg_tiles(id:vec2<u32>,size:vec2<u32>,s:FgSurface,sh:bool)->f32 {
    var total=0.0;
    let tiles=vec2<i32>((size+vec2<u32>(FG_TILE-1u))/FG_TILE);
    let f=(vec2<f32>(id)+vec2<f32>(0.5))/f32(FG_TILE)-vec2<f32>(0.5);
    let base=vec2<i32>(floor(f));
    let fr=f-floor(f);
    for(var k=0u;k<4u*FG_SLOTS;k+=1u) {
        let o=vec2<i32>(i32(k&1u),i32((k>>1u)&1u));
        let t=clamp(base+o,vec2<i32>(0),tiles-vec2<i32>(1));
        let index=fg_filtered(u32(t.y*tiles.x+t.x)*FG_SLOTS+k/4u,size);
        let bilinear=select(1.0-fr.x,fr.x,o.x!=0)*select(1.0-fr.y,fr.y,o.y!=0);
        let w=max(bilinear,1e-3)*fg_on_plane(fg_plane(index,size),s.world,s.normal);
        if w<=1e-6 {continue;}
        if sh {fg_add(index,size,w);}
        total+=w;
    }
    return total;
}
// One thread per tile: both slots' filtered copies. A neighbour weighs its agreement with the
// probe's plane (fg_on_plane at the probe's point and normal), a Gaussian in tile distance and
// its frames of history; the probe itself weighs its own frames.
const FG_FILTER_SHORT:f32=4.0;
@compute @workgroup_size(8,4) fn fg_filter(@builtin(global_invocation_id) grid:vec3<u32>) {
    let size=textureDimensions(primary_hit);
    let tiles=vec2<i32>((size+vec2<u32>(FG_TILE-1u))/FG_TILE);
    let own=vec2<i32>(grid.xy);
    if any(own>=tiles) {return;}
    for(var slot=0u;slot<FG_SLOTS;slot+=1u) {
        let index=u32(own.y*tiles.x+own.x)*FG_SLOTS+slot;
        let c=fg_plane(index,size);
        if c.point.w<=0.0 {fg_invalidate(fg_filtered(index,size),size);continue;}
        let reach=select(1,2,c.point.w<=FG_FILTER_SHORT);
        let sigma=f32(reach)*0.75;
        fg_clear();
        fg_add(index,size,c.point.w);
        var total=c.point.w;
        for(var y=-reach;y<=reach;y+=1) {
            for(var x=-reach;x<=reach;x+=1) {
                let t=own+vec2<i32>(x,y);
                if any(t<vec2<i32>(0)) || any(t>=tiles) {continue;}
                for(var k=0u;k<FG_SLOTS;k+=1u) {
                    let at=u32(t.y*tiles.x+t.x)*FG_SLOTS+k;
                    if at==index {continue;}
                    let p=fg_plane(at,size);
                    let w=fg_on_plane(p,c.point.xyz,c.normal.xyz)*exp(-f32(x*x+y*y)/(2.0*sigma*sigma))*p.point.w;
                    if w<=1e-4 {continue;}
                    fg_add(at,size,w);
                    total+=w;
                }
            }
        }
        var out:FgProbe;
        out.point=c.point;out.normal=c.normal;
        for(var v=0u;v<FG_SH_VECS;v+=1u) {out.sh[v]=fg_sum[v]/total;}
        fg_store(fg_filtered(index,size),size,out);
    }
}
var<workgroup> fg_orphan:atomic<u32>;
const FG_NONE:u32=0xffffffffu;
// The adaptive probe's pick for this frame: its pixel's lane in the block (bits 0-5), or
// FG_NONE, kept in the pad word after the SH (sh[6].w, which no reader takes as a coefficient).
fn fg_pick_at(index:u32,size:vec2<u32>)->u32 { return gi_prepared[fg_at(index,size)+2u].r0.w; }
// One workgroup per FG_BLOCK block, with no shading in it: the pixels without tile probes on
// their plane vote (in an order the frame turns) for the one the block's adaptive probe goes on.
// A block without one leaves its probe invalid.
@compute @workgroup_size(8,8) fn fg_classify(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_id) local:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let size=textureDimensions(primary_hit);
    let frame=fg_frame();
    if lane==0u {atomicStore(&fg_orphan,FG_NONE);}
    workgroupBarrier();
    let q=group.xy*FG_BLOCK+local.xy;
    let mine=fg_surface_at(q,size,false);
    if mine.placement!=0u && fg_tiles(q,size,mine,false)<=1e-4 {
        // 17 is odd, so the turned order is a permutation of the 64 lanes.
        atomicMin(&fg_orphan,(((lane*17u+frame*29u)&63u)<<6u)|lane);
    }
    workgroupBarrier();
    if lane!=0u {return;}
    let pick=atomicLoad(&fg_orphan);
    let index=fg_adaptive(group.xy,size);
    let at=fg_at(index,size)+2u;
    let c=gi_prepared[at];
    let stored=select(pick&63u,FG_NONE,pick==FG_NONE);
    gi_prepared[at]=GiPrepared(vec4<u32>(c.r0.xyz,stored),c.r1,c.r2);
    if pick==FG_NONE {fg_invalidate(index,size);}
}
var<workgroup> fg_pair:array<u32,2>;
// Two blocks per workgroup of 32 (fg_classify picked their pixels): lanes 16 b .. 16 b + 15
// trace block b's probe; a pair with no pick exits before any shading.
@compute @workgroup_size(32) fn fg_adapt(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let size=textureDimensions(primary_hit);
    let blocks=(size+vec2<u32>(FG_BLOCK-1u))/FG_BLOCK;
    let frame=fg_frame();
    let b=lane/FG_RAYS;
    let block=vec2<u32>(group.x*2u+b,group.y);
    let inside=all(block<blocks);
    let index=fg_adaptive(min(block,blocks-vec2<u32>(1u)),size);
    if lane%FG_RAYS==0u {fg_pair[b]=select(FG_NONE,fg_pick_at(index,size),inside);atomicStore(&fg_asked[b],0u);atomicStore(&fg_rays[b],0u);}
    let pair=workgroupUniformLoad(&fg_pair);
    if pair[0]==FG_NONE && pair[1]==FG_NONE {return;}
    let pick=pair[b];
    var s:FgSurface;
    var pixel=vec2<u32>(0u);
    if pick!=FG_NONE {
        pixel=block*FG_BLOCK+vec2<u32>(pick%FG_BLOCK,pick/FG_BLOCK);
        s=fg_surface_at(pixel,size,false);
    }
    // Half rate, as the tile probes (fg_trace): on alternate frames a checkerboard of blocks keeps
    // a probe that has history and still lies on the plane under its pick, instead of tracing it.
    let old=fg_plane(index,size);
    let keep=FG_HALF_RATE && ((block.x+block.y+frame)&1u)==1u && s.placement!=0u && old.point.w>=FG_HALF_MIN
        && dot(old.normal.xyz,s.normal)>0.9 && abs(dot(old.normal.xyz,s.world-old.point.xyz))<0.5*FG_PLANE*old.normal.w;
    var traced=s;
    if keep {traced.placement=0u;}
    fg_lane(lane,lane%FG_RAYS,traced,pixel,fg_hash(index^(frame*0x27d4eb2fu)));
    workgroupBarrier();
    fg_reduce(lane);
    workgroupBarrier();
    fg_count_rays(lane);
    if lane%FG_RAYS!=0u || pick==FG_NONE || s.placement==0u || keep {return;}
    fg_accumulate(index,size,b*FG_RAYS,s,pixel);
}
@compute @workgroup_size(8,4) fn fg_resolve(@builtin(global_invocation_id) grid:vec3<u32>) {
    let id=grid.xy+primary_region.xy;
    let size=textureDimensions(primary_hit);
    let s=fg_surface(id,size);
    if s.placement==0u {return;}
    fg_clear();
    var total=fg_tiles(id,size,s,true);
    if total<=1e-4 {
        // No tile probe on this plane: the adaptive probes around it, bilinear by block.
        let blocks=vec2<i32>((size+vec2<u32>(FG_BLOCK-1u))/FG_BLOCK);
        let f=(vec2<f32>(id)+vec2<f32>(0.5))/f32(FG_BLOCK)-vec2<f32>(0.5);
        let base=vec2<i32>(floor(f));
        let fr=f-floor(f);
        for(var k=0u;k<4u;k+=1u) {
            let o=vec2<i32>(i32(k&1u),i32(k>>1u));
            let b=clamp(base+o,vec2<i32>(0),blocks-vec2<i32>(1));
            let index=fg_adaptive(vec2<u32>(b),size);
            let bilinear=select(1.0-fr.x,fr.x,o.x!=0)*select(1.0-fr.y,fr.y,o.y!=0);
            let w=max(bilinear,1e-3)*fg_on_plane(fg_plane(index,size),s.world,s.normal);
            if w<=1e-6 {continue;}
            fg_add(index,size,w);
            total+=w;
        }
    }
    if total<=1e-4 {
        // None around on this plane: the best tile probe on it within FG_SEARCH tiles; failing
        // that, the facing probe of its own tile or block nearest its plane.
        let tiles=vec2<i32>((size+vec2<u32>(FG_TILE-1u))/FG_TILE);
        let own=vec2<i32>(id/FG_TILE);
        var best=0.0;
        var chosen=0u;
        for(var y=-FG_SEARCH;y<=FG_SEARCH;y+=1) {
            for(var x=-FG_SEARCH;x<=FG_SEARCH;x+=1) {
                let t=own+vec2<i32>(x,y);
                if any(t<vec2<i32>(0)) || any(t>=tiles) {continue;}
                for(var slot=0u;slot<FG_SLOTS;slot+=1u) {
                    let index=fg_filtered(u32(t.y*tiles.x+t.x)*FG_SLOTS+slot,size);
                    let w=fg_on_plane(fg_plane(index,size),s.world,s.normal)/f32(1+max(abs(x),abs(y)));
                    if w>best {best=w;chosen=index;}
                }
            }
        }
        if best<=0.05 {
            best=0.0;
            for(var k=0u;k<FG_SLOTS+1u;k+=1u) {
                let index=select(fg_filtered(u32(own.y*tiles.x+own.x)*FG_SLOTS+k,size),fg_adaptive(id/FG_BLOCK,size),k==FG_SLOTS);
                let p=fg_plane(index,size);
                if p.point.w<=0.0 {continue;}
                let facing=dot(p.normal.xyz,s.normal);
                let w=max(facing,0.0)/(1.0+abs(dot(p.normal.xyz,s.world-p.point.xyz))/max(p.normal.w,1e-6));
                if w>best {best=w;chosen=index;}
            }
        }
        if best>0.0 {fg_add(chosen,size,1.0);}
        total=select(0.0,1.0,best>0.0);
    }
    let inv=select(0.0,1.0/total,total>0.0);
    for(var i=0u;i<9u;i+=1u) {
        let a=i*3u;
        gi_rx_sky[i]=vec3<f32>(fg_sum[a/4u][a%4u],fg_sum[(a+1u)/4u][(a+1u)%4u],fg_sum[(a+2u)/4u][(a+2u)%4u])*inv;
    }
    for(var k=0u;k<4u;k+=1u) {
        let a=FG_BOUNCE+k*3u;
        gi_rx_bounce[k]=vec3<f32>(fg_sum[a/4u][a%4u],fg_sum[(a+1u)/4u][(a+1u)%4u],fg_sum[(a+2u)/4u][(a+2u)%4u])*inv;
    }
    gi_rx_valid=true;
    gi_rx_absolute=true;
    gi_rx_normal=s.shading;
    gi_rx_store_at(id.y*size.x+id.x);
}
