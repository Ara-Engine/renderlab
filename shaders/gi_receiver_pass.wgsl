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
const FG_RAYS:u32=32u;
const FG_HISTORY:f32=64.0;
// Half-rate tracing (off by default: fg_trace is ~0.4 ms on RT at 32 rays and full-rate updates
// eliminate half/quarter-rate checkerboard lag and speed up temporal convergence after cuts and motion).
const FG_HALF_RATE:bool=false;
const FG_HALF_MIN:f32=8.0;
// A probe ray's cone: its share of the hemisphere (2 pi / FG_HEMI sr) as an angle, so its
// width at distance t is FG_CONE * t (the cache answers hits whose cone covers a cell).
const FG_CONE:f32=0.45764912225414734;
// The sky a probe ray misses into: the map's own radiance in that direction (its base level).
// The probe's rays and history average it over the hemisphere. A lookup prefiltered to the ray's
// cone (GGX alpha 0.098) blurred the bright haze below the horizon into the directions above it:
// with every ray forced to miss, probes integrated 10.3% more than the open sky's irradiance
// (gi_open_irradiance) on Glint's floor, the sky part of the gather's overshoot in sky-lit shadow.
// At alpha 0 they integrate it within 0.01%. The whole hemisphere's blur, before that, had mixed
// the horizon into the zenith and greyed sky-lit shadow.
const FG_SKY_ALPHA:f32=0.0;
const FG_SEARCH:i32=2;
// A probe's plane reaches this fraction of its footprint along its normal on flat surfaces,
// relaxing slightly on smoothly curved surfaces (where chord sag scales with sin(theta)).
const FG_PLANE:f32=0.12;
// The incoming radiance as the lighting cache splits it, so shading reads a probe as it reads a
// cell (the sky's visibility for specular, es_prepare, and the bounce lobe of glossy surfaces,
// gi_rx_bounce_radiance): floats 0-26 the sky the probe's rays missed into (L2), 27 a pad, 28-39
// everything they met and the emitters (L1). Then the probe point (w: frames of history, 0 when
// invalid) and normal (w: the tile footprint in metres). Probes live after the pixels' records
// in gi_prepared, four records a probe: the tile probes, one adaptive probe per FG_BLOCK block,
// then the tile probes' filtered copies (src/gi_receiver_pass.rs sizes it).
// Probes live after the pixels' records in gi_prepared, five records a probe: sh (Sky L2 +
// Indirect Bounce L1), point/normal, and direct (Direct Analytical Emitter L1, accumulated on its
// own fast schedule so moving emitters have zero lag without wiping indirect bounce history).
// sh[6].w holds direct history count on stored probes (and adaptive pixel pick between fg_classify
// and fg_adapt on write_bank).
const FG_SH_VECS:u32=10u;
const FG_BOUNCE:u32=28u;
struct FgProbe { sh:array<vec4<f32>,10>, direct:array<vec4<f32>,3>, point:vec4<f32>, normal:vec4<f32> };
fn fg_at(index:u32,size:vec2<u32>)->u32 { return size.x*size.y+5u*index; }
// Double-buffered unfiltered probes (bank 0 and bank 1) followed by one filtered copy of the
// current frame's tile probes, so fg_trace and fg_adapt can reproject from the previous frame's
// bank without racing writes to the current frame's bank.
fn fg_bank_stride(size:vec2<u32>)->u32 {
    let tiles=(size+vec2<u32>(FG_TILE-1u))/FG_TILE;
    let blocks=(size+vec2<u32>(FG_BLOCK-1u))/FG_BLOCK;
    return tiles.x*tiles.y*FG_SLOTS+blocks.x*blocks.y;
}
fn fg_write_bank()->u32 { return fg_state[2]&1u; }
fn fg_read_bank()->u32 { return 1u-(fg_state[2]&1u); }
fn fg_tile_at(tile_slot:u32,size:vec2<u32>,bank:u32)->u32 {
    return bank*fg_bank_stride(size)+tile_slot;
}
fn fg_adaptive_in(block:vec2<u32>,size:vec2<u32>,bank:u32)->u32 {
    let tiles=(size+vec2<u32>(FG_TILE-1u))/FG_TILE;
    let blocks=(size+vec2<u32>(FG_BLOCK-1u))/FG_BLOCK;
    return bank*fg_bank_stride(size)+tiles.x*tiles.y*FG_SLOTS+block.y*blocks.x+block.x;
}
fn fg_filtered1(tile_slot:u32,size:vec2<u32>)->u32 {
    return 2u*fg_bank_stride(size)+tile_slot;
}
fn fg_filtered(tile_slot:u32,size:vec2<u32>)->u32 {
    let tiles=(size+vec2<u32>(FG_TILE-1u))/FG_TILE;
    return 2u*fg_bank_stride(size)+tiles.x*tiles.y*FG_SLOTS+tile_slot;
}
// Packs direct emitter history count [0..64] and log2(unshadowed Lambertian irradiance) into sh[6].w
// (2x f16, zero extra probe bytes) so temporal reprojection and spatial filtering demodulate 1/r^2
// and cos(theta) emitter falloff cleanly while denoising penumbra visibility.
fn fg_pack_direct_meta(dcount:f32,unsh:f32)->f32 {
    let lu=select(-30.0,clamp(log2(unsh),-28.0,28.0),unsh>1e-9);
    return bitcast<f32>(pack2x16float(vec2<f32>(clamp(dcount,0.0,64.0),lu)));
}
fn fg_unpack_direct_count(word:u32)->f32 { return unpack2x16float(word).x; }
fn fg_unpack_direct_unsh(word:u32)->f32 {
    let lu=unpack2x16float(word).y;
    return select(0.0,exp2(lu),lu>-29.0);
}
fn fg_probe_direct_luma(index:u32,size:vec2<u32>,n:vec3<f32>)->f32 {
    let d=gi_prepared[fg_at(index,size)+4u];
    return fg_l1_irradiance(array<vec4<f32>,3>(bitcast<vec4<f32>>(d.r0),bitcast<vec4<f32>>(d.r1),bitcast<vec4<f32>>(d.r2)),n);
}
fn fg_load(index:u32,size:vec2<u32>)->FgProbe {
    let at=fg_at(index,size);
    let a=gi_prepared[at];let b=gi_prepared[at+1u];let c=gi_prepared[at+2u];let e=gi_prepared[at+3u];let d=gi_prepared[at+4u];
    return FgProbe(array<vec4<f32>,10>(bitcast<vec4<f32>>(a.r0),bitcast<vec4<f32>>(a.r1),bitcast<vec4<f32>>(a.r2),
        bitcast<vec4<f32>>(b.r0),bitcast<vec4<f32>>(b.r1),bitcast<vec4<f32>>(b.r2),bitcast<vec4<f32>>(c.r0),
        bitcast<vec4<f32>>(c.r1),bitcast<vec4<f32>>(c.r2),bitcast<vec4<f32>>(e.r0)),
        array<vec4<f32>,3>(bitcast<vec4<f32>>(d.r0),bitcast<vec4<f32>>(d.r1),bitcast<vec4<f32>>(d.r2)),
        bitcast<vec4<f32>>(e.r1),bitcast<vec4<f32>>(e.r2));
}
// A probe's point and normal alone (the last two words of its fourth record): the plane tests
// read these, and the SH loads only once a weight is known (fg_add).
struct FgPlane { point:vec4<f32>, normal:vec4<f32> };
// The weighted SH sums fg_add / fg_add_both / fg_add_combined add to (private, not a pointer
// argument: naga's SPIR-V writer fails on a pointer to a local struct's member).
var<private> fg_sum:array<vec4<f32>,10>;
var<private> fg_direct_sum:array<vec4<f32>,3>;
fn fg_clear() {
    for(var v=0u;v<FG_SH_VECS;v+=1u) {fg_sum[v]=vec4<f32>(0.0);}
    for(var v=0u;v<3u;v+=1u) {fg_direct_sum[v]=vec4<f32>(0.0);}
}
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
fn fg_add_direct(index:u32,size:vec2<u32>,w:f32) {
    let d=gi_prepared[fg_at(index,size)+4u];
    fg_direct_sum[0]+=w*bitcast<vec4<f32>>(d.r0);
    fg_direct_sum[1]+=w*bitcast<vec4<f32>>(d.r1);
    fg_direct_sum[2]+=w*bitcast<vec4<f32>>(d.r2);
}
fn fg_add_both(index:u32,size:vec2<u32>,w:f32) {
    fg_add(index,size,w);
    fg_add_direct(index,size,w);
}
fn fg_add_combined(index:u32,size:vec2<u32>,w:f32) {
    fg_add(index,size,w);
    let d=gi_prepared[fg_at(index,size)+4u];
    fg_sum[7]+=w*bitcast<vec4<f32>>(d.r0);
    fg_sum[8]+=w*bitcast<vec4<f32>>(d.r1);
    fg_sum[9]+=w*bitcast<vec4<f32>>(d.r2);
}
fn fg_store(index:u32,size:vec2<u32>,p:FgProbe) {
    let at=fg_at(index,size);
    gi_prepared[at]=GiPrepared(bitcast<vec4<u32>>(p.sh[0]),bitcast<vec4<u32>>(p.sh[1]),bitcast<vec4<u32>>(p.sh[2]));
    gi_prepared[at+1u]=GiPrepared(bitcast<vec4<u32>>(p.sh[3]),bitcast<vec4<u32>>(p.sh[4]),bitcast<vec4<u32>>(p.sh[5]));
    gi_prepared[at+2u]=GiPrepared(bitcast<vec4<u32>>(p.sh[6]),bitcast<vec4<u32>>(p.sh[7]),bitcast<vec4<u32>>(p.sh[8]));
    gi_prepared[at+3u]=GiPrepared(bitcast<vec4<u32>>(p.sh[9]),bitcast<vec4<u32>>(p.point),bitcast<vec4<u32>>(p.normal));
    gi_prepared[at+4u]=GiPrepared(bitcast<vec4<u32>>(p.direct[0]),bitcast<vec4<u32>>(p.direct[1]),bitcast<vec4<u32>>(p.direct[2]));
}
fn fg_invalidate(index:u32,size:vec2<u32>) {
    let at=fg_at(index,size)+3u;
    gi_prepared[at]=GiPrepared(gi_prepared[at].r0,vec4<u32>(0u),vec4<u32>(0u));
}
// Per lane: its ray's terms in the probe's layout (sky L2 at 0-26 for a miss, bounce L1 at
// FG_BOUNCE for a hit, and 1 at FG_ANSWERED, the layout's pad, when the ray counts), and its
// emitter sample's bounce L1 terms (fg_nee).
var<workgroup> fg_lane_sh:array<array<f32,40>,64>;
var<workgroup> fg_nee_sh:array<array<f32,12>,64>;
const FG_ANSWERED:u32=27u;

fn fg_hash(x:u32)->u32 {
    var h=x*0x9e3779b1u;h^=h>>16u;h*=0x85ebca6bu;h^=h>>13u;h*=0xc2b2ae35u;h^=h>>16u;return h;
}
// A covered pixel's receiver for the gather: point, previous-frame world point (for motion
// reprojection), viewed geometric normal, shading normal, trace epsilon, placement + 1 (0: none),
// and whether the placement moved or is skinned this frame.
struct FgSurface { world:vec3<f32>, prev_world:vec3<f32>, normal:vec3<f32>, shading:vec3<f32>, epsilon:f32, placement:u32, moved:bool };
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
    let local_p=receiver_point(a.position,b.position,c.position,w);
    s.world=world_position(local_p,hit.placement).xyz;
    let half=arrayLength(&models)/2u;
    var rigid_moved=false;
    s.prev_world=s.world;
    if hit.placement<half {
        let cur_m=models[hit.placement];
        let prev_m=models[half+hit.placement];
        let eps=vec4<f32>(1e-5);
        rigid_moved=any(abs(cur_m[0]-prev_m[0])>eps)
            || any(abs(cur_m[1]-prev_m[1])>eps)
            || any(abs(cur_m[2]-prev_m[2])>eps)
            || any(abs(cur_m[3]-prev_m[3])>eps);
        if rigid_moved { s.prev_world=(prev_m*vec4<f32>(local_p,1.0)).xyz; }
    }
    s.moved=rigid_moved || batch_skinned(primary_batch_info(hit.batch));
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
// How far a point and normal lie on a probe's plane: symmetric plane distance with a hard
// cutoff beyond two widths so parallel steps/ledges (e.g. 0.25 m stair risers) never bleed from
// the tread above, curvature-scaled relaxation for smooth curved geometry, and cos^8 normal
// weighting with a 0.70 cutoff to prevent front-face cylinder probes from leaking onto grazing
// limbs. Returns 1 on the plane and 0 off it or for an invalid probe.
fn fg_on_plane(p:FgPlane,world:vec3<f32>,n:vec3<f32>)->f32 {
    if p.point.w<=0.0 {return 0.0;}
    let facing=dot(p.normal.xyz,n);
    if facing<=0.82 {return 0.0;}
    let d=world-p.point.xyz;
    let pdist=max(abs(dot(p.normal.xyz,d)),abs(dot(n,d)));
    let curve=sqrt(max(1.0-facing*facing,0.0));
    let scale=max((FG_PLANE+0.25*curve)*p.normal.w,1e-6);
    let plane=pdist/scale;
    if plane>=2.0 {return 0.0;}
    let f2=facing*facing;
    let f4=f2*f2;
    return exp(-plane*plane)*f4*f4;
}
// Screen-projected tile distance between two co-planar points on plane `n`, correcting for
// grazing foreshortening along the view-projected tangent axis (clamped at 1/min_cos). Front-facing
// surfaces equal isotropic length(delta)/scale; grazing walls and floors stop collapsing
// reprojection and spatial filtering along the foreshortened axis.
fn fg_aniso_dist(delta:vec3<f32>,n:vec3<f32>,point:vec3<f32>,scale:f32,min_cos:f32)->f32 {
    let v=lighting.eye-point;
    let v2=max(dot(v,v),1e-12);
    let vn=dot(v,n);
    let view_cos=max(abs(vn)*inverseSqrt(v2),min_cos);
    let proj=v-n*vn;
    let p2=dot(proj,proj);
    if p2<=1e-10 {return length(delta)/max(scale,1e-6);}
    let u=proj*inverseSqrt(p2);
    let d_par=dot(delta,u);
    let d_perp=delta-u*d_par;
    let scaled_par=d_par*view_cos;
    return sqrt(scaled_par*scaled_par+dot(d_perp,d_perp))/max(scale,1e-6);
}
fn fg_basis(n:vec3<f32>)->mat3x3<f32> {
    let s=select(-1.0,1.0,n.z>=0.0);
    let a=-1.0/(s+n.z);let b=n.x*n.y*a;
    return mat3x3<f32>(vec3<f32>(1.0+s*n.x*n.x*a,s*b,-s*n.x),vec3<f32>(b,s+n.y*n.y*a,-n.y),n);
}
// Aperiodic 2x2-stratified spatial base phase in [0, 1)^2: every 2x2 tile quad partitions
// [0, 1) into 4 complementary quarters (cancelling locally inside fg_filter), while each 2x2
// quad receives an independent Cranley-Patterson hash offset and sub-quad permutation, eliminating
// 4x4 (64-pixel) periodic stripes and horizontal walking waves.
fn fg_phase2_base(tile_xy:vec2<u32>,salt:u32)->vec2<f32> {
    let q=tile_xy>>vec2<u32>(1u);
    let sub=tile_xy&vec2<u32>(1u);
    let h=fg_hash((q.x*0x9e3779b1u)^(q.y*0x85ebca6bu)^salt);
    let h2=fg_hash(h^0x27d4eb2fu);
    let cp=vec2<f32>(f32(h&0xffffu),f32(h2&0xffffu))*(1.0/65536.0);
    let idx=(sub.x^(sub.y<<1u))^((h>>16u)&3u);
    let idx2=((sub.y^(sub.x<<1u))+((h2>>16u)&3u))&3u;
    let strat=vec2<f32>(f32(idx)*0.25,f32(idx2)*0.25);
    return fract(cp+strat);
}
fn fg_phase2(tile_xy:vec2<u32>,salt:u32)->vec2<f32> {
    let t=f32(fg_frame()&1023u);
    return fract(fg_phase2_base(tile_xy,salt)+vec2<f32>(0.754877666,0.569840296)*t);
}
// Blended cosine-weighted hemisphere direction k of `count`, stratified across neighboring tiles
// via `fg_phase2`: concentrates ~1.7x more rays into the normal lobe where Lambertian irradiance
// lives while keeping a 0.30 uniform floor (finite 3.33x horizon weight cap, 100% unbiased for L2
// sky and L1 bounce SH). Returns (direction.xyz, weight = 1 / (2*pi * pdf)).
const FG_COS_ALPHA:f32=0.70;
fn fg_direction(n:vec3<f32>,k:u32,count:u32,tile_xy:vec2<u32>)->vec4<f32> {
    let seq=fg_phase2(tile_xy,0x13579bdfu);
    let u=(f32(k)+fract(seq.x+f32(k)*0.3819660113))/f32(count);
    let a=FG_COS_ALPHA;
    let b=1.0-a;
    let z=clamp((-b+sqrt(max(b*b+4.0*a*u,0.0)))/(2.0*a),0.0,1.0);
    let w=1.0/(b+2.0*a*z);
    let rr=sqrt(max(1.0-z*z,0.0));
    let phi=6.283185307179586*fract(f32(k)*0.6180339887498949+seq.y);
    return vec4<f32>(fg_basis(n)*vec3<f32>(rr*cos(phi),rr*sin(phi),z),w);
}
// Incoming radiance (unexposed) along d from `origin`. A hit reads its lighting-cache cell's
// outgoing radiance, the only hit path: cell shading ran the hit's own program there, so the
// value carries that program's emission and albedo (Glint's white surfaces glow; Outpost's
// terrain and Ara's materials have their own programs) with the cell's indirect and direct
// light. The gather's module holds only the standard program, so shading the hit itself lost
// all of that: a probe hit on Glint's white surfaces came out without their glow, a tenth dark
// in sky-lit shadow. rl_cache_cell picks the cell (reflection_cache_hits.wgsl); a hit no level
// answers (or any hit while RENDERLAB_RL_CACHE_HITS=off leaves the stub in) sends nothing.
fn fg_radiance(origin:vec3<f32>,d:vec3<f32>,epsilon:f32,hemi_pdf:f32)->vec3<f32> {
    let traced=rl_cast(origin,d,epsilon);
    fg_hit=traced.record.x!=0u;
    fg_unanswered=false;
    fg_no_value=false;
    fg_changing=false;
    if traced.record.x==0u {return rl_sky(d,FG_SKY_ALPHA)/max(lighting.exposure,1e-30);}
    let cell=rl_cache_cell(traced.record,origin,d,traced.t,FG_CONE*traced.t);
    fg_touch(rl_cache_slots.x);
    fg_touch(rl_cache_slots.y);
    fg_no_value=cell.w==0.0;
    fg_unanswered=fg_no_value;
    fg_changing=cell.w>0.0 && cell.w<0.9;
    // A ray that meets an emitting draw keeps its cell's reflected light. When the scene's
    // emitter list is handled analytically by fg_direct_emitters_lane (n <= FG_DIRECT_EMITTERS),
    // emission is excluded from hemisphere rays entirely (zero double counting and zero MIS
    // noise); for huge emitter meshes (n > FG_DIRECT_EMITTERS), combine via MIS with fg_nee.
    let hit=primary_decode(traced.record);
    let emission=gi_triangle_emission(hit.batch,hit.placement,traced.record.y);
    if all(emission<=vec3<f32>(0.0)) {return rl_finite(cell.rgb);}
    // The triangle's emission is known exactly from the list, so the hit counts even when its
    // cell has no value (thin emitting strips' fine cells are often orphans that never get one);
    // only its reflected light, small on Glint's dark emitter bases, is then missing.
    fg_unanswered=false;
    let reflected=select(vec3<f32>(0.0),max(cell.rgb-emission,vec3<f32>(0.0)),cell.w>0.0);
    if fg_direct_emitters() {return rl_finite(reflected);}
    let light=fg_light_pdf(fg_luminance(emission),traced.t,abs(dot(unit_normal(rl_cache_geometric),d)));
    return rl_finite(reflected+emission*fg_mis(hemi_pdf*f32(FG_HEMI),light*f32(FG_NEE)));
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
// Whether its hit's cache cell is still converging / ungathered (w < 1.0 from rl_cache_cell).
var<private> fg_changing:bool;
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
// Header words in fg_state ahead of the per-probe pairs:
//   0: flags (bit 0: recently_changed / active movers; bit 1: emitter_moved)
//   1: gather_frame
//   2: bank parity (0 or 1; write bank = parity, read bank = 1 - parity)
//   3: has_prev (1 when words 4..20 hold a valid previous-frame unjittered view-projection)
//   4..20: previous unjittered view-projection mat4x4<f32> (column-major bitcast u32)
const FG_STATE_HEAD:u32=20u;
@group(2) @binding(31) var<storage,read_write> fg_state:array<u32>;
@group(2) @binding(32) var<storage,read_write> fg_depth:array<f32>;
fn fg_recently_changed()->bool { return (fg_state[0]&1u)!=0u; }
fn fg_emitter_moved()->bool { return (fg_state[0]&2u)!=0u; }
fn fg_state_at(index:u32)->u32 { return FG_STATE_HEAD+2u*index; }
fn fg_previous_vp()->mat4x4<f32> {
    return mat4x4<f32>(
        vec4<f32>(bitcast<f32>(fg_state[4]),bitcast<f32>(fg_state[5]),bitcast<f32>(fg_state[6]),bitcast<f32>(fg_state[7])),
        vec4<f32>(bitcast<f32>(fg_state[8]),bitcast<f32>(fg_state[9]),bitcast<f32>(fg_state[10]),bitcast<f32>(fg_state[11])),
        vec4<f32>(bitcast<f32>(fg_state[12]),bitcast<f32>(fg_state[13]),bitcast<f32>(fg_state[14]),bitcast<f32>(fg_state[15])),
        vec4<f32>(bitcast<f32>(fg_state[16]),bitcast<f32>(fg_state[17]),bitcast<f32>(fg_state[18]),bitcast<f32>(fg_state[19])),
    );
}
// Projects a world point into previous-frame pixel coordinates (z > 0 when inside the screen).
fn fg_reproject_pixel(world:vec3<f32>,size:vec2<u32>)->vec3<f32> {
    if fg_state[3]==0u {return vec3<f32>(0.0,0.0,-1.0);}
    let clip=fg_previous_vp()*vec4<f32>(world,1.0);
    if clip.w<=1e-6 {return vec3<f32>(0.0,0.0,-1.0);}
    let ndc=clip.xy/clip.w;
    if any(abs(ndc)>vec2<f32>(1.0)) {return vec3<f32>(0.0,0.0,-1.0);}
    let uv=vec2<f32>(ndc.x*0.5+0.5,0.5-ndc.y*0.5);
    return vec3<f32>(uv*vec2<f32>(size),1.0);
}
// Best matching previous-frame tile probe in read_bank for surface `s` (0xffffffffu if none).
// Weighted strictly by plane + 3D world-space closeness (never by history length) so static
// probes stay anchored to their own world position instead of advecting across planar walls.
fn fg_find_prev_tile(s:FgSurface,size:vec2<u32>,footprint:f32)->u32 {
    let rp=fg_reproject_pixel(s.prev_world,size);
    if rp.z<=0.0 {return 0xffffffffu;}
    let tiles=vec2<i32>((size+vec2<u32>(FG_TILE-1u))/FG_TILE);
    let center=vec2<i32>(floor(rp.xy/f32(FG_TILE)));
    let bank=fg_read_bank();
    var best=0.0;
    var chosen=0xffffffffu;
    for(var oy=-1;oy<=1;oy+=1) {
        for(var ox=-1;ox<=1;ox+=1) {
            let t=clamp(center+vec2<i32>(ox,oy),vec2<i32>(0),tiles-vec2<i32>(1));
            for(var slot=0u;slot<FG_SLOTS;slot+=1u) {
                let idx=fg_tile_at(u32(t.y*tiles.x+t.x)*FG_SLOTS+slot,size,bank);
                var p=fg_plane(idx,size);
                if p.point.w<=0.0 || dot(p.normal.xyz,s.normal)<0.95 {continue;}
                let scale=max(footprint,p.normal.w);
                p.normal.w=scale;
                let geo=fg_on_plane(p,s.prev_world,s.normal);
                if geo<=0.01 {continue;}
                let d3=fg_aniso_dist(s.prev_world-p.point.xyz,s.normal,s.world,scale,0.125);
                if d3>1.75 {continue;}
                let score=geo*exp(-d3*d3*3.0);
                if score>best {best=score;chosen=idx;}
            }
        }
    }
    return chosen;
}
// Co-planar edge-preserving bilinear reprojection of a previous-frame tile probe for `s`. On still
// cameras where the probe stays on its exact previous world point, returns `fg_load(prev_idx)` with
// zero blur and full history; during camera motion/jumps, interpolates the 2x2 co-planar previous
// tiles and measures the world-space centroid shift (`ext_shift`) so one-sided horizon edges (where
// a wall rejects and only foreground floor remains) never smear foreground radiance into corners.
fn fg_load_prev_tile(s:FgSurface,size:vec2<u32>,footprint:f32,prev_idx:u32,cur_unsh:f32)->FgProbe {
    var empty:FgProbe;
    if prev_idx==0xffffffffu {return empty;}
    let anchor=fg_plane(prev_idx,size);
    let anchor_shift=fg_aniso_dist(s.prev_world-anchor.point.xyz,s.normal,s.world,footprint,0.125);
    if anchor_shift<=0.05 {
        return fg_load(prev_idx,size);
    }
    let rp=fg_reproject_pixel(s.prev_world,size);
    if rp.z<=0.0 {return empty;}
    let tiles=vec2<i32>((size+vec2<u32>(FG_TILE-1u))/FG_TILE);
    // Tile `t` places its probe at pixel `t * FG_TILE + (8, 8)` (center `t * FG_TILE + 8.5`).
    let f=(rp.xy-vec2<f32>(0.5))/f32(FG_TILE)-vec2<f32>(0.5);
    let base=vec2<i32>(floor(f));
    let fr=f-floor(f);
    let bank=fg_read_bank();
    fg_clear();
    var w_sum=0.0;
    var wd_sum=0.0;
    var hist_sum=0.0;
    var dhist_sum=0.0;
    var unsh_sum=0.0;
    var pos_sum=vec3<f32>(0.0);
    var ndot_sum=0.0;
    for(var k=0u;k<4u;k+=1u) {
        let o=vec2<i32>(i32(k&1u),i32((k>>1u)&1u));
        let t=clamp(base+o,vec2<i32>(0),tiles-vec2<i32>(1));
        let bil=select(1.0-fr.x,fr.x,o.x!=0)*select(1.0-fr.y,fr.y,o.y!=0);
        if bil<=1e-4 {continue;}
        // Pick the single best co-planar slot at this bilinear corner and weight strictly by
        // `bil * geo` (no Gaussian distance penalty fighting `bil`, guaranteeing zero phase lag).
        var best_geo=0.0;
        var best_idx=0xffffffffu;
        var best_p:FgPlane;
        var best_ndot=0.0;
        for(var slot=0u;slot<FG_SLOTS;slot+=1u) {
            let idx=fg_tile_at(u32(t.y*tiles.x+t.x)*FG_SLOTS+slot,size,bank);
            var p=fg_plane(idx,size);
            let ndot=dot(p.normal.xyz,s.normal);
            if p.point.w<=0.0 || ndot<0.95 {continue;}
            let scale=max(footprint,p.normal.w);
            p.normal.w=scale;
            let geo=fg_on_plane(p,s.prev_world,s.normal);
            if geo<=0.01 {continue;}
            let d3=fg_aniso_dist(s.prev_world-p.point.xyz,s.normal,s.world,scale,0.125);
            if d3>1.75 {continue;}
            if geo>best_geo {best_geo=geo;best_idx=idx;best_p=p;best_ndot=ndot;}
        }
        if best_idx==0xffffffffu {continue;}
        let w=bil*best_geo;
        if w<=1e-5 {continue;}
        fg_add(best_idx,size,w);
        w_sum+=w;
        hist_sum+=w*best_p.point.w;
        pos_sum+=w*best_p.point.xyz;
        ndot_sum+=w*best_ndot;
        let dmeta=gi_prepared[fg_at(best_idx,size)+2u].r0.w;
        let p_dc=fg_unpack_direct_count(dmeta);
        let p_unsh=fg_unpack_direct_unsh(dmeta);
        if p_dc>0.0 && (cur_unsh<=1e-7 || p_unsh>1e-7) {
            let d_scale=select(1.0,clamp(cur_unsh/p_unsh,0.25,4.0),cur_unsh>1e-7 && p_unsh>1e-7);
            fg_add_direct(best_idx,size,w*d_scale);
            wd_sum+=w;
            dhist_sum+=w*p_dc;
            unsh_sum+=w*p_unsh;
        }
    }
    let max_ext=0.95;
    if w_sum<=1e-4 {
        if anchor_shift>max_ext {return empty;}
        var fb=fg_load(prev_idx,size);
        let curved=dot(fb.normal.xyz,s.normal)<0.995;
        let ext_t=smoothstep(0.35,max_ext,anchor_shift);
        fb.point.w=min(fb.point.w,mix(select(8.0,4.0,curved),select(1.5,3.0,FG_HALF_RATE),ext_t));
        let dmeta=bitcast<u32>(fb.sh[6].w);
        let p_dc=fg_unpack_direct_count(dmeta);
        let p_unsh=fg_unpack_direct_unsh(dmeta);
        if cur_unsh>1e-7 && p_unsh>1e-7 {
            let d_scale=clamp(cur_unsh/p_unsh,0.25,4.0);
            for(var v=0u;v<3u;v+=1u) {fb.direct[v]*=d_scale;}
        }
        let kept_dc=select(min(p_dc,1.0),0.0,cur_unsh>1e-7 && p_unsh<=1e-7);
        fb.sh[6].w=fg_pack_direct_meta(kept_dc,select(p_unsh,cur_unsh,cur_unsh>1e-7));
        return fb;
    }
    let inv=1.0/w_sum;
    let ext_shift=fg_aniso_dist(s.prev_world-pos_sum*inv,s.normal,s.world,footprint,0.125);
    if ext_shift>max_ext {return empty;}
    var out:FgProbe;
    for(var v=0u;v<FG_SH_VECS;v+=1u) {out.sh[v]=fg_sum[v]*inv;}
    let inv_d=select(0.0,1.0/wd_sum,wd_sum>1e-5);
    for(var v=0u;v<3u;v+=1u) {out.direct[v]=fg_direct_sum[v]*inv_d;}
    let blur=fr.x*(1.0-fr.x)+fr.y*(1.0-fr.y);
    let blur_t=clamp(blur*2.0,0.0,1.0);
    let ext_t=smoothstep(0.35,max_ext,ext_shift);
    let curved=ndot_sum*inv<0.995;
    var move_cap=mix(mix(16.0,8.0,blur_t),select(1.5,3.0,FG_HALF_RATE),ext_t);
    if curved {move_cap=min(move_cap,4.0);}
    let kept_dc=select(0.0,min(dhist_sum*inv_d,1.0),wd_sum>1e-5);
    let kept_unsh=select(unsh_sum*inv_d,cur_unsh,cur_unsh>1e-7);
    out.sh[6].w=fg_pack_direct_meta(kept_dc,kept_unsh);
    out.point=vec4<f32>(s.prev_world,min(hist_sum*inv,move_cap));
    out.normal=vec4<f32>(s.normal,footprint);
    return out;
}
// Best matching previous-frame adaptive (or tile fallback) probe in read_bank for surface `s`.
fn fg_find_prev_adapt(s:FgSurface,size:vec2<u32>,footprint:f32)->u32 {
    let rp=fg_reproject_pixel(s.prev_world,size);
    if rp.z<=0.0 {return 0xffffffffu;}
    let blocks=vec2<i32>((size+vec2<u32>(FG_BLOCK-1u))/FG_BLOCK);
    let base=vec2<i32>(floor(rp.xy/f32(FG_BLOCK)-vec2<f32>(0.5)));
    let bank=fg_read_bank();
    var best=0.0;
    var chosen=0xffffffffu;
    for(var k=0u;k<4u;k+=1u) {
        let o=vec2<i32>(i32(k&1u),i32(k>>1u));
        let b=clamp(base+o,vec2<i32>(0),blocks-vec2<i32>(1));
        let idx=fg_adaptive_in(vec2<u32>(b),size,bank);
        var p=fg_plane(idx,size);
        if p.point.w<=0.0 || dot(p.normal.xyz,s.normal)<0.95 {continue;}
        let full_scale=max(footprint,p.normal.w);
        p.normal.w=full_scale;
        let geo=fg_on_plane(p,s.prev_world,s.normal);
        if geo<=0.01 {continue;}
        let d3=fg_aniso_dist(s.prev_world-p.point.xyz,s.normal,s.world,0.5*full_scale,0.125);
        if d3>1.75 {continue;}
        let score=geo*exp(-d3*d3*4.0);
        if score>best {best=score;chosen=idx;}
    }
    if chosen!=0xffffffffu {return chosen;}
    return fg_find_prev_tile(s,size,footprint);
}
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
var<workgroup> fg_wg_cold:array<atomic<u32>,2>;
var<workgroup> fg_wg_geom:array<atomic<u32>,2>;
var<workgroup> fg_wg_geom_ans:array<atomic<u32>,2>;
var<workgroup> fg_wg_unsh:array<f32,2>;
var<workgroup> fg_trace_mask:array<u32,2>;
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
// Analytical polygon solid-angle emitter evaluation for scenes with up to FG_DIRECT_EMITTERS
// listed emitter triangles (light panels, room fixtures, lamps): exact horizon-clipped SH
// (`gi_emitter_sh`) with Bayer+R2 stratified visibility rays across all FG_RAYS lanes, giving
// zero unshadowed variance on frame 1 and crisp, blotch-free emitter shadows. Huge emitter
// meshes (n > FG_DIRECT_EMITTERS) fall back to stratified MIS NEE (`fg_nee`).
const FG_DIRECT_EMITTERS:u32=64u;
fn fg_direct_emitters()->bool {
    let n=gi_word(GI_H_EMITTER_COUNT);
    return n>0u && n<=FG_DIRECT_EMITTERS;
}
fn fg_direct_emitters_lane(lane:u32,ray:u32,s:FgSurface,tile_xy:vec2<u32>) {
    let n=gi_word(GI_H_EMITTER_COUNT);
    if n==0u {return;}
    let base=gi_word(GI_H_EMITTERS);
    let origin=fma(s.normal,vec3<f32>(s.epsilon),s.world);
    let base_seq=fg_phase2_base(tile_xy,0x2468ace1u);
    let frame_idx=fg_frame()&1023u;
    // Pass 1 (uniform across the probe's warp): identify visible emitter triangles, peak irradiance,
    // and exact total analytical unshadowed Lambertian irradiance (for visibility demodulation so
    // temporal/spatial filtering never blurs 1/r^2 or cos(theta) emitter falloff).
    var vis_lo=0u;
    var vis_hi=0u;
    var vis_count=0u;
    var max_irrad=0.0;
    var unsh_total=0.0;
    for(var e=0u;e<n;e+=1u) {
        let r=base+e*GI_EMITTER_WORDS;
        let radiance=gi_emitter_vec(r+9u);
        if all(radiance<=vec3<f32>(0.0)) {continue;}
        let a=gi_emitter_vec(r);
        let e1=gi_emitter_vec(r+3u);
        let e2=gi_emitter_vec(r+6u);
        let sh=gi_emitter_sh(s.world,s.normal,a,a+e1,a+e2);
        if sh.x<=2.8e-7 {continue;}
        let lum=fg_luminance(radiance);
        let irrad=lum*sh.x;
        let lamb=max(sh.x*0.886227+(sh.y*s.normal.y+sh.z*s.normal.z+sh.w*s.normal.x)*1.023327,0.0);
        unsh_total+=lum*lamb;
        max_irrad=max(max_irrad,irrad);
        if e<32u {vis_lo|=1u<<e;} else {vis_hi|=1u<<(e-32u);}
        vis_count+=1u;
    }
    if ray==0u {fg_wg_unsh[lane/FG_RAYS]=unsh_total;}
    if vis_count==0u {return;}
    // Classify major (>=15% of peak irradiance) vs minor visible emitter triangles: major faces
    // share the full warp (FG_RAYS / major_count, up to 16 R2-stratified rays/triangle) without
    // losing lanes to tiny box-edge slivers, while minor faces still trace 1 unbiased ray each.
    var maj_lo=0u;
    var maj_hi=0u;
    var major_count=0u;
    let thresh=max_irrad*0.15;
    for(var e=0u;e<n;e+=1u) {
        let on=select((vis_hi&(1u<<(e-32u)))!=0u,(vis_lo&(1u<<e))!=0u,e<32u);
        if !on {continue;}
        let r=base+e*GI_EMITTER_WORDS;
        let a=gi_emitter_vec(r);
        let e1=gi_emitter_vec(r+3u);
        let e2=gi_emitter_vec(r+6u);
        let sh=gi_emitter_sh(s.world,s.normal,a,a+e1,a+e2);
        if fg_luminance(gi_emitter_vec(r+9u))*sh.x>=thresh {
            if e<32u {maj_lo|=1u<<e;} else {maj_hi|=1u<<(e-32u);}
            major_count+=1u;
        }
    }
    let major_spp=select(1u,clamp(FG_RAYS/max(major_count,1u),1u,16u),major_count>0u);
    var slot=0u;
    for(var e=0u;e<n;e+=1u) {
        let on=select((vis_hi&(1u<<(e-32u)))!=0u,(vis_lo&(1u<<e))!=0u,e<32u);
        if !on {continue;}
        let is_major=select((maj_hi&(1u<<(e-32u)))!=0u,(maj_lo&(1u<<e))!=0u,e<32u);
        let spp=select(1u,major_spp,is_major);
        let start=slot;
        slot+=spp;
        // Find which sample(s) in [start, start + spp) belong to this lane (`idx % FG_RAYS == ray`).
        var first=start;
        let rem=start%FG_RAYS;
        if ray>=rem {first=start+(ray-rem);} else {first=start+(FG_RAYS-rem+ray);}
        if first>=slot {continue;}
        let r=base+e*GI_EMITTER_WORDS;
        let a=gi_emitter_vec(r);
        let e1=gi_emitter_vec(r+3u);
        let e2=gi_emitter_vec(r+6u);
        let radiance=gi_emitter_vec(r+9u);
        let sh=gi_emitter_sh(s.world,s.normal,a,a+e1,a+e2);
        let inv_spp=1.0/f32(spp);
        let tri_shift=vec2<f32>(0.3819660113,0.6180339887)*f32(e);
        for(var idx=first;idx<slot;idx+=FG_RAYS) {
            let sub=idx-start;
            let seq=fract(base_seq+tri_shift+vec2<f32>(0.754877666,0.569840296)*f32(frame_idx*spp+sub));
            var uv=seq;
            if uv.x+uv.y>1.0 {uv=vec2<f32>(1.0)-uv;}
            var x=a+e1*uv.x+e2*uv.y;
            if dot(x-s.world,s.normal)<=1e-5 {
                x=a+(e1+e2)*(1.0/3.0);
                if dot(x-s.world,s.normal)<=1e-5 {
                    let h0=dot(a-s.world,s.normal);
                    let h1=dot(a+e1-s.world,s.normal);
                    let h2=dot(a+e2-s.world,s.normal);
                    x=select(select(a+e2,a+e1,h1>=h2),a,h0>=h1 && h0>=h2);
                }
            }
            let to=x-origin;
            let dist=length(to);
            if dist<=1e-6 {continue;}
            atomicAdd(&fg_rays[1],1u);
            if rl_blocked(origin,to/dist,s.epsilon,dist*0.999) {continue;}
            let c0=radiance*(sh.x*inv_spp);
            let c1=radiance*(sh.y*inv_spp);
            let c2=radiance*(sh.z*inv_spp);
            let c3=radiance*(sh.w*inv_spp);
            fg_nee_sh[lane][0u]+=c0.x;fg_nee_sh[lane][1u]+=c0.y;fg_nee_sh[lane][2u]+=c0.z;
            fg_nee_sh[lane][3u]+=c1.x;fg_nee_sh[lane][4u]+=c1.y;fg_nee_sh[lane][5u]+=c1.z;
            fg_nee_sh[lane][6u]+=c2.x;fg_nee_sh[lane][7u]+=c2.y;fg_nee_sh[lane][8u]+=c2.z;
            fg_nee_sh[lane][9u]+=c3.x;fg_nee_sh[lane][10u]+=c3.y;fg_nee_sh[lane][11u]+=c3.z;
        }
    }
}
// Traces lane `ray` of a probe at s and stores its SH terms in fg_lane_sh[lane]: a miss's as the
// sky's L2, a hit's as the bounce's L1 (zero when the probe has no surface).
fn fg_lane(lane:u32,ray:u32,s:FgSurface,pixel:vec2<u32>,r:u32,tile_xy:vec2<u32>) {
    var radiance=vec3<f32>(0.0);
    var d=vec3<f32>(0.0,0.0,1.0);
    var ray_w=1.0;
    fg_hit=false;
    var answered=false;
    for(var i=0u;i<40u;i+=1u) {fg_lane_sh[lane][i]=0.0;}
    for(var i=0u;i<12u;i+=1u) {fg_nee_sh[lane][i]=0.0;}
    let direct=fg_direct_emitters();
    let hemi_count=select(FG_HEMI,FG_RAYS,direct);
    if s.placement!=0u {
        if !direct && ray<FG_NEE {
            // Large emitter meshes: stratified emitter sample takes this lane's one ray (fg_nee).
            fg_nee(lane,ray,s,fg_hash(r^(fg_frame()*0x27d4eb2fu)),tile_xy);
        } else {
            let hemi_ray=select(ray-FG_NEE,ray,direct);
            let dir_w=fg_direction(s.normal,hemi_ray,hemi_count,tile_xy);
            d=dir_w.xyz;
            ray_w=dir_w.w;
            let hemi_pdf=FG_HEMISPHERE_PDF/max(ray_w,1e-6);
            radiance=fg_radiance(fma(s.normal,vec3<f32>(s.epsilon),s.world),d,s.epsilon,hemi_pdf);
            atomicAdd(&fg_rays[0],1u);
            if fg_hit && fg_no_value && rl_cache_missed.x!=0xffffffffu && atomicAdd(&fg_asked[lane/FG_RAYS],1u)<4u {fg_request(rl_cache_missed);}
            if fg_hit {
                atomicAdd(&fg_wg_geom[lane/FG_RAYS],1u);
                if !fg_unanswered {atomicAdd(&fg_wg_geom_ans[lane/FG_RAYS],1u);}
            }
            if fg_hit && (fg_unanswered || fg_changing) {atomicAdd(&fg_wg_cold[lane/FG_RAYS],1u);}
            // A hit no cache level answers yet is left out of the bounce average (fg_accumulate
            // renormalizes answered bounce over geometry hits alone) while sky rays always keep
            // their exact 2*pi/hemi_count solid-angle share so missing cache cells never over-brighten sky light.
            answered=!(fg_hit && fg_unanswered);
        }
        if direct {
            fg_direct_emitters_lane(lane,ray,s,tile_xy);
        }
    }
    if !answered {return;}
    gi_sh_basis(d);
    let weighted=radiance*ray_w;
    let sky=select(weighted,vec3<f32>(0.0),fg_hit);
    let bounce=select(vec3<f32>(0.0),weighted,fg_hit);
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
// by multiple importance sampling when the scene has more than FG_DIRECT_EMITTERS triangles.
// Stores absolute L1 SH directly in fg_nee_sh (matching fg_direct_emitters_lane).
const FG_NEE:u32=2u;
const FG_HEMI:u32=FG_RAYS-FG_NEE;
fn fg_nee(lane:u32,ray:u32,s:FgSurface,r:u32,tile_xy:vec2<u32>) {
    let phase=fg_phase2(tile_xy,0x369cf258u);
    let u_pick=fract((f32(ray)+phase.x)/f32(FG_NEE));
    let pick=gi_emitter_pick(u_pick);
    if pick.probability<=0.0 {return;}
    let w=gi_word(GI_H_EMITTERS)+pick.index*GI_EMITTER_WORDS;
    let e1=gi_emitter_vec(w+3u);
    let e2=gi_emitter_vec(w+6u);
    let g=fg_hash(r^((ray+1u)*0x9e3779b9u));
    var uv=vec2<f32>(f32(g&0xffffu),f32(g>>16u))*(1.0/65536.0);
    if uv.x+uv.y>1.0 {uv=vec2<f32>(1.0)-uv;}
    let origin=fma(s.normal,vec3<f32>(s.epsilon),s.world);
    let to=gi_emitter_vec(w)+e1*uv.x+e2*uv.y-origin;
    let dist=length(to);
    if dist<=1e-6 {return;}
    let d=to/dist;
    let z_e=dot(d,s.normal);
    if z_e<=0.0 {return;}
    let side=cross(e1,e2);
    let area=0.5*length(side);
    let cosine=abs(dot(side,d))*0.5/max(area,1e-30);
    if area<=0.0 || cosine<=1e-4 {return;}
    let density=pick.probability*dist*dist/(area*cosine);
    atomicAdd(&fg_rays[1],1u);
    if rl_blocked(origin,d,s.epsilon,dist*0.999) {return;}
    let hemi_pdf=FG_HEMISPHERE_PDF*((1.0-FG_COS_ALPHA)+2.0*FG_COS_ALPHA*z_e);
    let weight=fg_mis(density*f32(FG_NEE),hemi_pdf*f32(FG_HEMI));
    let light=gi_emitter_vec(w+9u)*(weight/(density*f32(FG_NEE)));
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
    let nee_lanes=select(FG_NEE,FG_RAYS,fg_direct_emitters());
    // Exposure- and ray-count-invariant lone-ray firefly clamp evaluated over bounce SH: compares
    // the hottest ray against the per-ray mean of the remaining rays (180x rest_mean), matching RT
    // and non-RT (16 vs 32 rays) identically without over-darkening legitimate non-RT bounce pools.
    var total_luma=0.0;
    var max_luma=0.0;
    var max_lane=0u;
    for(var l=0u;l<FG_RAYS;l+=1u) {
        let c=vec3<f32>(fg_lane_sh[first+l][FG_BOUNCE],fg_lane_sh[first+l][FG_BOUNCE+1u],fg_lane_sh[first+l][FG_BOUNCE+2u]);
        let y=dot(max(c,vec3<f32>(0.0)),vec3<f32>(0.2126,0.7152,0.0722));
        total_luma+=y;
        if y>max_luma {max_luma=y;max_lane=l;}
    }
    let rest_mean=max(total_luma-max_luma,0.0)/max(f32(FG_RAYS-1u),1.0);
    let firefly_cap=180.0*rest_mean;
    let cap_scale=select(1.0,min(1.0,firefly_cap/max(max_luma,1e-30)),max_luma>firefly_cap && rest_mean>1e-7);
    workgroupBarrier();
    for(var i=lane%FG_RAYS;i<52u;i+=FG_RAYS) {
        var sum=0.0;
        if i<40u {
            for(var l=0u;l<FG_RAYS;l+=1u) {
                let w=select(1.0,cap_scale,l==max_lane && i>=FG_BOUNCE);
                sum+=fg_lane_sh[first+l][i]*w;
            }
            fg_lane_sh[first][i]=sum;
        } else {
            for(var l=0u;l<nee_lanes;l+=1u) {sum+=fg_nee_sh[first+l][i-40u];}
            fg_nee_sh[first][i-40u]=sum;
        }
    }
}
// Luminance of an L1 SH signal (direct analytical emitters) evaluated at normal `n`.
fn fg_l1_irradiance(l1:array<vec4<f32>,3>,n:vec3<f32>)->f32 {
    gi_sh_basis(n);
    var e=vec3<f32>(0.0);
    for(var i=0u;i<4u;i+=1u) {
        let a=i*3u;
        e+=gi_band_weight(i)*gi_y[i]*vec3<f32>(l1[a/4u][a%4u],l1[(a+1u)/4u][(a+1u)%4u],l1[(a+2u)/4u][(a+2u)%4u]);
    }
    return dot(max(e,vec3<f32>(0.0)),vec3<f32>(0.2126,0.7152,0.0722));
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
// Smooth, variance- and log-ratio-normalized history adaptation (`fg_state`): compares this
// update against the probe's running noise level `usual` AND unbounded EV-stop change `log_diff`
// (so transient motion raising `usual` can never blind a genuine 4x-50x lighting drop). Normal
// Monte Carlo noise (`z <= 1.8`) keeps full 64-frame history; real lighting, shadow, or bounce
// changes smoothly lower the history cap down to `FG_RESTART` with zero binary contour rings.
const FG_CHANGE_LO:f32=1.8;
const FG_CHANGE_HI:f32=4.5;
const FG_RESTART:f32=1.5;
fn fg_adapt_history(index:u32,fresh:array<vec4<f32>,10>,old:FgProbe,count:f32,max_hist:f32,n:vec3<f32>,missing_frac:f32)->f32 {
    let at=fg_state_at(index);
    if at+1u>=arrayLength(&fg_state) {return max_hist;}
    let word=fg_state[at];
    var usual=bitcast<f32>(fg_state[at+1u]);
    if count<=1.0 || missing_frac>0.35 {
        fg_state[at]=word&0xffffff00u;
        fg_state[at+1u]=bitcast<u32>(select(0.12,0.18,FG_HALF_RATE));
        return max_hist;
    }
    let e_new=fg_irradiance(fresh,n);
    let e_old=fg_irradiance(old.sh,n);
    let peak=max(e_new,e_old);
    if peak<=1e-4 {
        let quiet=select(word&0x3fu,min((word&0x3fu)+1u,63u),old.point.w>=FG_HALF_MIN);
        fg_state[at]=(word&0xffffff00u)|quiet;
        return max_hist;
    }
    let r=abs(e_new-e_old)/peak;
    let log_diff=abs(log2(max(e_new,1e-4))-log2(max(e_old,1e-4)));
    let z=max(r/max(usual,0.05),log_diff*2.5);
    let decay=smoothstep(FG_CHANGE_LO,FG_CHANGE_HI,z);
    let sign_bit=select(0u,128u,e_new>e_old);
    let prev_pending=(word&64u)!=0u;
    let prev_sign=word&128u;
    let confirmed=prev_pending && prev_sign==sign_bit;
    if old.point.w>=FG_RESTART && decay>0.0 {
        if confirmed || z>=3.0 {
            fg_state[at]=((fg_frame()&0xffffffu)<<8u)|64u|sign_bit;
            return mix(max_hist,FG_RESTART,decay);
        }
        fg_state[at]=(word&0xffffff3fu)|64u|sign_bit;
        if z>=2.3 {
            return mix(max_hist,FG_RESTART,decay*0.35);
        }
        return max_hist;
    }
    usual=clamp(mix(usual,r,0.125),0.03,0.18);
    let quiet=select(word&0x3fu,min((word&0x3fu)+1u,63u),old.point.w>=FG_HALF_MIN);
    fg_state[at]=(word&0xffffff00u)|quiet;
    fg_state[at+1u]=bitcast<u32>(usual);
    return max_hist;
}
// Folds the lanes from `first` (summed into that row by fg_reduce) into probe `write_index`
// (in write_bank), continuing from `prev_index` (in read_bank, 0xffffffffu when none) and
// recording change state at `state_index`.
fn fg_accumulate(write_index:u32,prev_index:u32,state_index:u32,size:vec2<u32>,probe_slot:u32,s:FgSurface,pixel:vec2<u32>,bilinear_tile:bool,probe_shifted:bool) {
    let first=probe_slot*FG_RAYS;
    var sh:array<f32,40>;
    var nee:array<f32,12>;
    // fg_reduce left each sum in the probe's first row.
    for(var i=0u;i<40u;i+=1u) {sh[i]=fg_lane_sh[first][i];}
    for(var i=0u;i<12u;i+=1u) {nee[i]=fg_nee_sh[first][i];}
    let footprint=fg_footprint(pixel,size,s.world);
    let direct=fg_direct_emitters();
    let cur_unsh=select(0.0,fg_wg_unsh[probe_slot],direct);
    let has_old=prev_index!=0xffffffffu;
    var old:FgProbe;
    if has_old {
        if bilinear_tile {old=fg_load_prev_tile(s,size,footprint,prev_index,cur_unsh);}
        else {
            old=fg_load(prev_index,size);
            let ashift=fg_aniso_dist(s.prev_world-old.point.xyz,s.normal,s.world,footprint,0.125);
            if ashift>0.95 {
                old.point.w=0.0;
                old.sh[6].w=fg_pack_direct_meta(0.0,cur_unsh);
            } else if ashift>0.05 {
                let curved=dot(old.normal.xyz,s.normal)<0.995;
                let ext_t=smoothstep(0.35,0.95,ashift);
                old.point.w=min(old.point.w,mix(select(14.0,4.0,curved),select(1.5,2.5,FG_HALF_RATE),ext_t));
                let dmeta=bitcast<u32>(old.sh[6].w);
                let p_dc=fg_unpack_direct_count(dmeta);
                let p_unsh=fg_unpack_direct_unsh(dmeta);
                if cur_unsh>1e-7 && p_unsh>1e-7 {
                    let d_scale=clamp(cur_unsh/p_unsh,0.25,4.0);
                    for(var v=0u;v<3u;v+=1u) {old.direct[v]*=d_scale;}
                }
                var d_cap=mix(10.0,select(1.5,2.5,FG_HALF_RATE),ext_t);
                if curved {d_cap=min(d_cap,4.0);}
                let kept_dc=select(min(p_dc,d_cap),0.0,cur_unsh>1e-7 && p_unsh<=1e-7);
                old.sh[6].w=fg_pack_direct_meta(kept_dc,select(p_unsh,cur_unsh,cur_unsh>1e-7));
            }
        }
    }
    let fresh_direct=array<vec4<f32>,3>(
        vec4<f32>(nee[0],nee[1],nee[2],nee[3]),
        vec4<f32>(nee[4],nee[5],nee[6],nee[7]),
        vec4<f32>(nee[8],nee[9],nee[10],nee[11]),
    );
    let answered=sh[FG_ANSWERED];
    let has_nee=(nee[0]+nee[1]+nee[2])>1e-6;
    // Zero-answer guard: when every hemisphere ray hit an unshaded cache cell (e.g. frame 0 in a
    // tight corner) and no direct emitter light arrived, preserve valid on-plane history or
    // invalidate rather than storing pitch-black. When direct emitters already arrived (e.g. night
    // light panels on frame 1), keep their exact irradiance immediately so frame 1 is never black.
    if answered<=0.0 && !has_nee {
        if has_old && old.point.w>0.0 {
            var kept=old;
            kept.point=vec4<f32>(s.world,old.point.w);
            kept.normal=vec4<f32>(s.normal,footprint);
            fg_store(write_index,size,kept);
        } else {
            fg_invalidate(write_index,size);
        }
        return;
    }
    // Uniform hemisphere: pdf 1/(2 pi). Sky rays ALWAYS carry their exact 2*pi / hemi_count share
    // so unanswered geometry hits never over-amplify sky light. When direct analytical emitters are
    // active (`direct`), they accumulate in `out.direct` on their own fast schedule instead of
    // being mixed into the 64-frame indirect bounce SH, giving zero lag when moving emitters
    // without wiping indirect bounce history or creating contour rings.
    let hemi_count=select(FG_HEMI,FG_RAYS,direct);
    let hemi_scale=6.283185307179586/f32(hemi_count);
    let missing_frac=clamp((f32(hemi_count)-answered)/f32(hemi_count),0.0,1.0);
    let geom_hits=atomicLoad(&fg_wg_geom[probe_slot]);
    let geom_ans=atomicLoad(&fg_wg_geom_ans[probe_slot]);
    let geom_missing_frac=select(0.0,clamp(f32(geom_hits-geom_ans)/f32(geom_hits),0.0,1.0),geom_hits>0u);
    let cold_frac=clamp(f32(atomicLoad(&fg_wg_cold[probe_slot]))/f32(hemi_count),0.0,1.0);
    sh[FG_ANSWERED]=0.0;
    for(var i=0u;i<FG_BOUNCE;i+=1u) {sh[i]*=hemi_scale;}
    if has_old && old.point.w>0.0 && geom_missing_frac>0.0 {
        for(var i=0u;i<12u;i+=1u) {
            let b=FG_BOUNCE+i;
            let add_nee=select(nee[i],0.0,direct);
            let rem=old.sh[b/4u][b%4u]-add_nee;
            let borrow=select(rem,max(rem,0.0),i<3u);
            sh[b]=sh[b]*hemi_scale+borrow*geom_missing_frac+add_nee;
        }
    } else if geom_hits>0u && geom_ans>0u {
        let bounce_scale=hemi_scale*min(f32(geom_hits)/f32(geom_ans),1.25);
        for(var i=0u;i<12u;i+=1u) {sh[FG_BOUNCE+i]=sh[FG_BOUNCE+i]*bounce_scale+select(nee[i],0.0,direct);}
    } else {
        for(var i=0u;i<12u;i+=1u) {sh[FG_BOUNCE+i]=sh[FG_BOUNCE+i]*hemi_scale+select(nee[i],0.0,direct);}
    }
    let emitter_moved=fg_emitter_moved();
    // Indirect bounce keeps 12 frames while emitters/lights are actively relighting so moving
    // panels never inject 4-frame hemisphere noise clouds while fg_adapt_history flushes real drops.
    let max_hist=select(select(FG_HISTORY,16.0,s.moved),12.0,emitter_moved || fg_recently_changed());
    var count=max(1.0-geom_missing_frac,0.25);
    if has_old && old.point.w>0.0 {
        count=min(old.point.w+1.0,max_hist);
    }
    var fresh:array<vec4<f32>,10>;
    for(var v=0u;v<FG_SH_VECS;v+=1u) {fresh[v]=vec4<f32>(sh[v*4u],sh[v*4u+1u],sh[v*4u+2u],sh[v*4u+3u]);}
    let hist_cap=fg_adapt_history(state_index,fresh,old,count,max_hist,s.normal,max(missing_frac,geom_missing_frac));
    count=min(count,hist_cap);
    let cold_t=smoothstep(0.25,0.65,cold_frac);
    let warm_eff=select(max(count-6.0,2.0),min(count,2.0),count<=8.0);
    let startup_cold=fg_recently_changed() || count<=6.0;
    let eff_count=mix(count,warm_eff,select(cold_t*0.35,cold_t,startup_cold));
    let blend=1.0/eff_count;
    var out:FgProbe;
    for(var v=0u;v<FG_SH_VECS;v+=1u) {out.sh[v]=select(fresh[v],mix(old.sh[v],fresh[v],blend),count>1.0);}
    // Separate direct analytical emitter accumulation (demodulated against cur_unsh):
    // - Still probes (`!probe_shifted`, `!emitter_moved`): full 24-frame zero-blur convergence in
    //   static soft penumbrae without Monte Carlo resets (and fast `d_rel` reset on dynamic scenes).
    // - Moving probes (`probe_shifted` or `emitter_moved`): fresh analytical direct samples every
    //   frame (16 rays RT / 8 rays Non-RT + 2-pass 9x9 co-planar binomial à-trous filter + pixel-
    //   rate TR1), avoiding chained 16-px bilinear shadow smear and lag on narrow shadows.
    var direct_count=0.0;
    if direct {
        if emitter_moved || probe_shifted {
            direct_count=1.0;
            out.direct=fresh_direct;
        } else {
            let old_dw=select(0.0,fg_unpack_direct_count(bitcast<u32>(old.sh[6].w)),has_old && old.point.w>0.0);
            let d_new=fg_l1_irradiance(fresh_direct,s.normal);
            let d_old=fg_l1_irradiance(old.direct,s.normal);
            let d_peak=max(d_new,d_old);
            let d_rel=select(0.0,abs(d_new-d_old)/d_peak,d_peak>1e-4);
            let d_max=select(24.0,mix(select(16.0,8.0,s.moved),1.5,smoothstep(0.25,0.65,d_rel)),fg_recently_changed() || s.moved);
            direct_count=select(1.0,min(old_dw+1.0,d_max),old_dw>0.0);
            let d_blend=1.0/direct_count;
            for(var k=0u;k<3u;k+=1u) {
                out.direct[k]=select(fresh_direct[k],mix(old.direct[k],fresh_direct[k],d_blend),direct_count>1.0);
            }
        }
    } else {
        out.direct=array<vec4<f32>,3>(vec4<f32>(0.0),vec4<f32>(0.0),vec4<f32>(0.0));
    }
    out.sh[6].w=fg_pack_direct_meta(direct_count,cur_unsh);
    out.point=vec4<f32>(s.world,count);
    out.normal=vec4<f32>(s.normal,footprint);
    fg_store(write_index,size,out);
}
@compute @workgroup_size(64) fn fg_trace(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let size=textureDimensions(primary_hit);
    let tiles=(size+vec2<u32>(FG_TILE-1u))/FG_TILE;
    let tile=group.y*tiles.x+group.x;
    let frame=fg_frame();
    let slot=lane/FG_RAYS;
    if lane<2u {atomicStore(&fg_asked[lane],0u);atomicStore(&fg_rays[lane],0u);atomicStore(&fg_wg_cold[lane],0u);atomicStore(&fg_wg_geom[lane],0u);atomicStore(&fg_wg_geom_ans[lane],0u);fg_wg_unsh[lane]=0.0;}
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
                if at<arrayLength(&fg_state) && ((fg_state[at]&64u)!=0u || fg_changed_recently(fg_state[at],frame)) {atomicOr(&fg_moved,1u);}
            }
        }
    }
    workgroupBarrier();
    // Slot 0: fixed center-anchored pick (no frame-to-frame pixel dither across polygon facet
    // seams on curved cylinders/pillars or thin rails), falling back to a 4x4 sweep if empty.
    var first:FgSurface;
    var first_pixel=vec2<u32>(0u);
    let q0=group.xy*FG_TILE+vec2<u32>(8u,8u);
    first=fg_surface_at(q0,size,false);
    if first.placement!=0u {
        first_pixel=q0;
    } else {
        for(var k=0u;k<16u;k+=1u) {
            let q=group.xy*FG_TILE+vec2<u32>((k&3u)*4u+2u,(k>>2u)*4u+2u);
            let c=fg_surface_at(q,size,false);
            if c.placement!=0u {first=c;first_pixel=q;break;}
        }
    }
    var s=first;
    var pixel=first_pixel;
    if slot==1u {
        s=FgSurface();
        if first.placement!=0u {
            var probe:FgPlane;
            probe.point=vec4<f32>(first.world,1.0);
            probe.normal=vec4<f32>(first.normal,fg_footprint(first_pixel,size,first.world));
            for(var k=0u;k<16u;k+=1u) {
                let q=group.xy*FG_TILE+vec2<u32>((k&3u)*4u+1u,(k>>2u)*4u+1u);
                let c=fg_surface_at(q,size,false);
                if c.placement!=0u && fg_on_plane(probe,c.world,c.normal)<0.37 {s=c;pixel=q;break;}
            }
        }
    }
    let tile_slot=tile*FG_SLOTS+slot;
    let write_idx=fg_tile_at(tile_slot,size,fg_write_bank());
    let footprint=select(0.0,fg_footprint(pixel,size,s.world),s.placement!=0u);
    let prev_idx=select(0xffffffffu,fg_find_prev_tile(s,size,footprint),s.placement!=0u);
    // Half rate: on alternate frames, a checkerboard of tiles carries forward a reprojected probe
    // that has history on its plane instead of tracing it. Drop out of stable 1/8 cadence when the
    // probe shifted across the surface (camera pan/motion) so moving walls stay crisp on non-RT.
    var prev_plane:FgPlane;
    if prev_idx!=0xffffffffu {prev_plane=fg_plane(prev_idx,size);}
    let old_w=prev_plane.point.w;
    let probe_shifted=prev_idx!=0xffffffffu && fg_aniso_dist(s.prev_world-prev_plane.point.xyz,s.normal,s.world,footprint,0.125)>0.05;
    let at=fg_state_at(tile_slot);
    let emitter_moved=fg_emitter_moved();
    let stable=!fg_recently_changed() && !emitter_moved && !probe_shifted && old_w>=28.0 && at<arrayLength(&fg_state) && (fg_state[at]&0x3fu)>=FG_STABLE && atomicLoad(&fg_moved)==0u;
    let turn=select((group.x+group.y+frame)&1u,(group.x+2u*group.y+frame)%FG_STABLE_RATE,stable);
    let moving_direct=probe_shifted && fg_direct_emitters();
    let keep=!emitter_moved && !fg_recently_changed() && !moving_direct && atomicLoad(&fg_moved)==0u && FG_HALF_RATE && turn!=0u && s.placement!=0u && old_w>=FG_HALF_MIN;
    var traced=s;
    if keep {traced.placement=0u;}
    if lane%FG_RAYS==0u {fg_trace_mask[slot]=u32(traced.placement!=0u);}
    let trace_mask=workgroupUniformLoad(&fg_trace_mask);
    if trace_mask[0]!=0u || trace_mask[1]!=0u {
        fg_lane(lane,lane%FG_RAYS,traced,pixel,fg_hash(tile_slot),group.xy);
        workgroupBarrier();
        fg_reduce(lane);
        workgroupBarrier();
        fg_count_rays(lane);
    }
    if lane%FG_RAYS!=0u || any(group.xy>=tiles) {return;}
    if s.placement==0u {fg_invalidate(write_idx,size);return;}
    if keep {
        var carried=fg_load(prev_idx,size);
        if probe_shifted {
            let bil=fg_load_prev_tile(s,size,footprint,prev_idx,0.0);
            if bil.point.w>0.0 {
                carried=bil;
            }
        }
        carried.point=vec4<f32>(s.world,carried.point.w);
        carried.normal=vec4<f32>(s.normal,footprint);
        fg_store(write_idx,size,carried);
        return;
    }
    fg_accumulate(write_idx,prev_idx,tile_slot,size,slot,s,pixel,true,probe_shifted);
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
// Fast DC luminance of a stored probe (sky L0 + bounce L0 + direct emitter L0) for edge-stopping.
fn fg_probe_dc_luma(index:u32,size:vec2<u32>)->f32 {
    let at=fg_at(index,size);
    let sky0=bitcast<vec4<f32>>(gi_prepared[at].r0).xyz;
    let bounce0=bitcast<vec4<f32>>(gi_prepared[at+2u].r1).xyz;
    let direct0=bitcast<vec4<f32>>(gi_prepared[at+4u].r0).xyz;
    return fg_luminance(max(sky0+bounce0+direct0,vec3<f32>(0.0)));
}
// Pass 1 of the two-pass co-planar probe filter (5x5 step-1 -> fg_filtered1): keeps indirect SH
// and demodulated direct emitter visibility separate for Pass 2 (fg_filter2, 3x3 step-2 ->
// fg_filtered). Uses anisotropic foreshortening-aware distance so grazing walls and floors filter
// cleanly in 2D without vertical column tears, and demodulates direct emitters by exact analytical
// unshadowed irradiance (c_unsh / p_unsh) so soft penumbrae denoise cleanly without blurring 1/r^2
// gradients or curved surfaces.
const FG_FILTER_SHORT:f32=24.0;
@compute @workgroup_size(8,4) fn fg_filter(@builtin(global_invocation_id) grid:vec3<u32>) {
    let size=textureDimensions(primary_hit);
    let tiles=vec2<i32>((size+vec2<u32>(FG_TILE-1u))/FG_TILE);
    let own=vec2<i32>(grid.xy);
    if any(own>=tiles) {return;}
    let bank=fg_write_bank();
    for(var slot=0u;slot<FG_SLOTS;slot+=1u) {
        let tile_slot=u32(own.y*tiles.x+own.x)*FG_SLOTS+slot;
        let index=fg_tile_at(tile_slot,size,bank);
        let c=fg_plane(index,size);
        if c.point.w<=0.0 {fg_invalidate(fg_filtered1(tile_slot,size),size);continue;}
        let c_meta=gi_prepared[fg_at(index,size)+2u].r0.w;
        let c_unsh=fg_unpack_direct_unsh(c_meta);
        let c_vis=select(0.0,clamp(fg_probe_direct_luma(index,size,c.normal.xyz)/c_unsh,0.0,1.0),c_unsh>1e-7);
        let age_t=clamp((c.point.w-1.0)/23.0,0.0,1.0);
        let sigma_sh=mix(select(1.45,1.75,FG_HALF_RATE),select(1.20,1.35,FG_HALF_RATE),age_t);
        // Keep direct spatial filter response consistent between still and moving probes so
        // diagonal emitter shadows never sawtooth when still or shift width when camera moves.
        let sigma_dir=select(0.95,1.10,FG_HALF_RATE);
        let reach=2;
        fg_clear();
        let sh_center_w=c.point.w;
        let dir_center_w=1.5;
        fg_add(index,size,sh_center_w);
        fg_add_direct(index,size,dir_center_w);
        var total_sh=sh_center_w;
        var total_dir=dir_center_w;
        for(var y=-reach;y<=reach;y+=1) {
            for(var x=-reach;x<=reach;x+=1) {
                if x==0 && y==0 {continue;}
                let t=own+vec2<i32>(x,y);
                if any(t<vec2<i32>(0)) || any(t>=tiles) {continue;}
                let r2=x*x+y*y;
                if abs(x)==2 || abs(y)==2 {
                    let mid=own+vec2<i32>(x/2,y/2);
                    var mid_ok=false;
                    for(var mk=0u;mk<FG_SLOTS;mk+=1u) {
                        let mat=fg_tile_at(u32(mid.y*tiles.x+mid.x)*FG_SLOTS+mk,size,bank);
                        if fg_on_plane(fg_plane(mat,size),c.point.xyz,c.normal.xyz)>0.02 {mid_ok=true;break;}
                    }
                    if !mid_ok {continue;}
                }
                let opp=own-vec2<i32>(x,y);
                let opp_in=all(opp>=vec2<i32>(0)) && all(opp<tiles);
                var opp_ok=false;
                if opp_in {
                    for(var ok=0u;ok<FG_SLOTS;ok+=1u) {
                        let oat=fg_tile_at(u32(opp.y*tiles.x+opp.x)*FG_SLOTS+ok,size,bank);
                        if fg_on_plane(fg_plane(oat,size),c.point.xyz,c.normal.xyz)>0.05 {opp_ok=true;break;}
                    }
                }
                let pair_sh=select(select(0.85,0.95,FG_HALF_RATE),1.0,opp_ok || !opp_in);
                let pair_dir_flat=select(0.10,1.0,opp_ok || !opp_in);
                let pair_dir_curved=select(0.10,1.0,opp_ok);
                let g_sh=exp(-f32(r2)/(2.0*sigma_sh*sigma_sh))*pair_sh;
                let g_dir=exp(-f32(r2)/(2.0*sigma_dir*sigma_dir))*pair_dir_flat;
                for(var k=0u;k<FG_SLOTS;k+=1u) {
                    let at=fg_tile_at(u32(t.y*tiles.x+t.x)*FG_SLOTS+k,size,bank);
                    if at==index {continue;}
                    let p=fg_plane(at,size);
                    let geo=fg_on_plane(p,c.point.xyz,c.normal.xyz);
                    if geo<=1e-4 {continue;}
                    let scale=max(c.normal.w,p.normal.w);
                    let d_sh=fg_aniso_dist(p.point.xyz-c.point.xyz,c.normal.xyz,c.point.xyz,scale,0.125);
                    let d_sh_excess=max(d_sh-select(1.0,2.0,FG_HALF_RATE),0.0);
                    let dist_sh=exp(-d_sh_excess*d_sh_excess*(1.0/16.0));
                    let sh_hist_w=min(p.point.w,max(sh_center_w*2.5,4.0));
                    let w_sh=geo*dist_sh*sh_hist_w*g_sh;
                    if w_sh>1e-4 {
                        fg_add(at,size,w_sh);
                        total_sh+=w_sh;
                    }
                    let flat_pair=dot(c.normal.xyz,p.normal.xyz)>=0.995;
                    let d_dir=fg_aniso_dist(p.point.xyz-c.point.xyz,c.normal.xyz,c.point.xyz,scale,0.20);
                    let dist_dir=exp(-d_dir*d_dir*(1.0/16.0));
                    if flat_pair {
                        if c_unsh>1e-7 {
                            let p_meta=gi_prepared[fg_at(at,size)+2u].r0.w;
                            let p_unsh=fg_unpack_direct_unsh(p_meta);
                            if p_unsh>1e-7 {
                                let p_vis=clamp(fg_probe_direct_luma(at,size,p.normal.xyz)/p_unsh,0.0,1.0);
                                let dv=p_vis-c_vis;
                                let vis_w=exp(-dv*dv*select(7.5,7.2,FG_HALF_RATE));
                                let u_ratio=c_unsh/p_unsh;
                                let u_log=abs(log2(clamp(u_ratio,0.125,8.0)));
                                let u_w=1.0-smoothstep(0.35,1.0,u_log);
                                let w_dir=geo*dist_dir*vis_w*u_w*min(p.point.w,1.0)*g_dir;
                                if w_dir>1e-4 {
                                    fg_add_direct(at,size,w_dir*u_ratio);
                                    total_dir+=w_dir;
                                }
                            }
                        }
                    } else if abs(x)<=1 && abs(y)<=1 {
                        // Curved surfaces (cylinders/spheres) smooth direct light across adjacent
                        // facets with a tight 3x3 geometric kernel without demodulation distortion.
                        let g_c=exp(-f32(r2)/(2.0*0.60*0.60))*pair_dir_curved;
                        let w_dir=geo*dist_dir*min(p.point.w,1.0)*g_c;
                        if w_dir>1e-4 {
                            fg_add_direct(at,size,w_dir);
                            total_dir+=w_dir;
                        }
                    }
                }
            }
        }
        var out:FgProbe;
        out.point=c.point;out.normal=c.normal;
        let inv_sh=1.0/max(total_sh,1e-6);
        let inv_dir=select(0.0,1.0/total_dir,total_dir>1e-6);
        for(var v=0u;v<FG_SH_VECS;v+=1u) {out.sh[v]=fg_sum[v]*inv_sh;}
        out.sh[6].w=bitcast<f32>(c_meta);
        for(var v=0u;v<3u;v+=1u) {out.direct[v]=fg_direct_sum[v]*inv_dir;}
        fg_store(fg_filtered1(tile_slot,size),size,out);
    }
}
// Pass 2 of the co-planar probe filter (3x3 at step 2 tiles -> fg_filtered): cascades with Pass 1
// to give a 9x9 co-planar denoiser (eliminating indirect hotspot clouds and soft penumbra noise
// on cuts, walking, and panning) while midpoint continuity checks and visibility edge-stopping
// preserve stair steps, corners, curved cylinders, and crisp emitter shadow boundaries.
@compute @workgroup_size(8,4) fn fg_filter2(@builtin(global_invocation_id) grid:vec3<u32>) {
    let size=textureDimensions(primary_hit);
    let tiles=vec2<i32>((size+vec2<u32>(FG_TILE-1u))/FG_TILE);
    let own=vec2<i32>(grid.xy);
    if any(own>=tiles) {return;}
    for(var slot=0u;slot<FG_SLOTS;slot+=1u) {
        let tile_slot=u32(own.y*tiles.x+own.x)*FG_SLOTS+slot;
        let index=fg_filtered1(tile_slot,size);
        let c=fg_plane(index,size);
        if c.point.w<=0.0 {fg_invalidate(fg_filtered(tile_slot,size),size);continue;}
        let c_meta=gi_prepared[fg_at(index,size)+2u].r0.w;
        let c_unsh=fg_unpack_direct_unsh(c_meta);
        let c_vis=select(0.0,clamp(fg_probe_direct_luma(index,size,c.normal.xyz)/c_unsh,0.0,1.0),c_unsh>1e-7);
        let c_sh_luma=fg_luminance(max(bitcast<vec4<f32>>(gi_prepared[fg_at(index,size)].r0).xyz+bitcast<vec4<f32>>(gi_prepared[fg_at(index,size)+2u].r1).xyz,vec3<f32>(0.0)));
        // Consistent step-2 filter sigmas across still and moving probes so bright near-field
        // bounce cliffs next to emitters never change width when camera motion starts or stops.
        let sigma_sh2=select(1.05,1.90,FG_HALF_RATE);
        let sigma_dir2=select(0.55,0.65,FG_HALF_RATE);
        fg_clear();
        let sh_center_w=c.point.w;
        let dir_center_w=1.0;
        fg_add(index,size,sh_center_w);
        fg_add_direct(index,size,dir_center_w);
        var total_sh=sh_center_w;
        var total_dir=dir_center_w;
        for(var dy=-1;dy<=1;dy+=1) {
            for(var dx=-1;dx<=1;dx+=1) {
                if dx==0 && dy==0 {continue;}
                let t=own+vec2<i32>(dx*2,dy*2);
                if any(t<vec2<i32>(0)) || any(t>=tiles) {continue;}
                let mid=own+vec2<i32>(dx,dy);
                var mid_ok=false;
                for(var mk=0u;mk<FG_SLOTS;mk+=1u) {
                    let mat=fg_filtered1(u32(mid.y*tiles.x+mid.x)*FG_SLOTS+mk,size);
                    if fg_on_plane(fg_plane(mat,size),c.point.xyz,c.normal.xyz)>0.02 {mid_ok=true;break;}
                }
                if !mid_ok {continue;}
                let opp=own-vec2<i32>(dx*2,dy*2);
                let opp_in=all(opp>=vec2<i32>(0)) && all(opp<tiles);
                var opp_ok=false;
                if opp_in {
                    for(var ok=0u;ok<FG_SLOTS;ok+=1u) {
                        let oat=fg_filtered1(u32(opp.y*tiles.x+opp.x)*FG_SLOTS+ok,size);
                        if fg_on_plane(fg_plane(oat,size),c.point.xyz,c.normal.xyz)>0.05 {opp_ok=true;break;}
                    }
                }
                let pair_sh=select(select(0.65,0.95,FG_HALF_RATE),1.0,opp_ok || !opp_in);
                let pair_dir=select(0.10,1.0,opp_ok || !opp_in);
                let r2=dx*dx+dy*dy;
                let g_sh=exp(-f32(r2)/(2.0*sigma_sh2*sigma_sh2))*pair_sh;
                let g_dir=exp(-f32(r2)/(2.0*sigma_dir2*sigma_dir2))*pair_dir;
                for(var k=0u;k<FG_SLOTS;k+=1u) {
                    let at=fg_filtered1(u32(t.y*tiles.x+t.x)*FG_SLOTS+k,size);
                    let p=fg_plane(at,size);
                    let geo=fg_on_plane(p,c.point.xyz,c.normal.xyz);
                    if geo<=1e-4 {continue;}
                    // Step-2 (32 px) pass only merges flat co-planar tiles (preserving curved
                    // cylinders) and edge-stops across steep near-field emitter bounce cliffs.
                    if dot(c.normal.xyz,p.normal.xyz)<0.995 {continue;}
                    let scale=max(c.normal.w,p.normal.w);
                    let d_sh=fg_aniso_dist(p.point.xyz-c.point.xyz,c.normal.xyz,c.point.xyz,scale,0.125);
                    let d_sh_excess=max(d_sh-2.0,0.0);
                    let dist_sh=exp(-d_sh_excess*d_sh_excess*(1.0/25.0));
                    let p_sh_luma=fg_luminance(max(bitcast<vec4<f32>>(gi_prepared[fg_at(at,size)].r0).xyz+bitcast<vec4<f32>>(gi_prepared[fg_at(at,size)+2u].r1).xyz,vec3<f32>(0.0)));
                    let sh_rel=abs(p_sh_luma-c_sh_luma)/max(max(p_sh_luma,c_sh_luma),1e-4);
                    let sh_ex=max(sh_rel-0.18,0.0);
                    let sh_lw=exp(-sh_ex*sh_ex*5.0);
                    let sh_hist_w=min(p.point.w,max(sh_center_w*2.0,4.0));
                    let w_sh=geo*dist_sh*sh_lw*sh_hist_w*g_sh;
                    if w_sh>1e-4 {
                        fg_add(at,size,w_sh);
                        total_sh+=w_sh;
                    }
                    if c_unsh>1e-7 {
                        let p_meta=gi_prepared[fg_at(at,size)+2u].r0.w;
                        let p_unsh=fg_unpack_direct_unsh(p_meta);
                        if p_unsh>1e-7 {
                            let p_vis=clamp(fg_probe_direct_luma(at,size,p.normal.xyz)/p_unsh,0.0,1.0);
                            let dv=p_vis-c_vis;
                            let vis_w=exp(-dv*dv*select(13.0,12.5,FG_HALF_RATE));
                            let u_ratio=c_unsh/p_unsh;
                            let u_log=abs(log2(clamp(u_ratio,0.125,8.0)));
                            let u_w=1.0-smoothstep(0.35,1.0,u_log);
                            let d_dir=fg_aniso_dist(p.point.xyz-c.point.xyz,c.normal.xyz,c.point.xyz,scale,0.20);
                            let dist_dir=exp(-d_dir*d_dir*(1.0/25.0));
                            let w_dir=geo*dist_dir*vis_w*u_w*g_dir;
                            if w_dir>1e-4 {
                                fg_add_direct(at,size,w_dir*u_ratio);
                                total_dir+=w_dir;
                            }
                        }
                    }
                }
            }
        }
        var out:FgProbe;
        out.point=c.point;out.normal=c.normal;
        let inv_sh=1.0/max(total_sh,1e-6);
        let inv_dir=select(0.0,1.0/total_dir,total_dir>1e-6);
        for(var v=0u;v<7u;v+=1u) {out.sh[v]=fg_sum[v]*inv_sh;}
        for(var v=0u;v<3u;v+=1u) {out.sh[7u+v]=fg_sum[7u+v]*inv_sh+fg_direct_sum[v]*inv_dir;}
        out.direct=array<vec4<f32>,3>(vec4<f32>(0.0),vec4<f32>(0.0),vec4<f32>(0.0));
        fg_store(fg_filtered(tile_slot,size),size,out);
    }
}
var<workgroup> fg_orphan:atomic<u32>;
const FG_NONE:u32=0xffffffffu;
// The adaptive probe's pick for this frame: its pixel's lane in the block (bits 0-5), or
// FG_NONE, kept in the pad word after the SH (sh[6].w, which no reader takes as a coefficient).
fn fg_pick_at(index:u32,size:vec2<u32>)->u32 { return gi_prepared[fg_at(index,size)+2u].r0.w; }
// Fast world-position reconstruction from a pixel's stored linear ray distance (`fg_depth`).
fn fg_world_at(p:vec2<u32>,size:vec2<u32>,dist:f32)->vec3<f32> {
    let ndc=fma((vec2<f32>(p)+vec2<f32>(0.5))/vec2<f32>(size),vec2<f32>(2.0,-2.0),vec2<f32>(-1.0,1.0));
    let inv=primary_frame.inverse;
    let xy=fma(inv[0],vec4<f32>(ndc.x),inv[1]*ndc.y);
    let a=xy+inv[2]+inv[3];
    let b=xy+inv[3];
    let near=a.xyz/a.w;
    let far=b.xyz/b.w;
    return near+normalize(far-near)*dist;
}
// Short-range sub-probe horizon ambient occlusion (cosine-weighted sin^2(h) integral over 8 fixed
// screen directions within ~1.8 tile footprints). Captures crisp concave corner darkening inside
// the probe-spacing blind spot without halos on distant edges or noise on static views.
fn fg_corner_ao(id:vec2<u32>,size:vec2<u32>,s:FgSurface)->f32 {
    let footprint=fg_footprint(id,size,s.world);
    let world_r=clamp(1.8*footprint,0.14,0.75);
    let r2=world_r*world_r;
    let screen_r=clamp(f32(size.y)*0.032,14.0,32.0);
    let lo=vec2<i32>(primary_region.xy);
    let hi=vec2<i32>(min(size,primary_region.zw))-vec2<i32>(1);
    let dirs=array<vec2<f32>,8>(
        vec2<f32>(1.0,0.0),vec2<f32>(0.70710678,0.70710678),
        vec2<f32>(0.0,1.0),vec2<f32>(-0.70710678,0.70710678),
        vec2<f32>(-1.0,0.0),vec2<f32>(-0.70710678,-0.70710678),
        vec2<f32>(0.0,-1.0),vec2<f32>(0.70710678,-0.70710678),
    );
    let fracs=array<f32,4>(0.14,0.34,0.62,1.0);
    var occ_sum=0.0;
    let origin=vec2<f32>(id);
    for(var k=0u;k<8u;k+=1u) {
        let dir=dirs[k];
        var h_max=0.0;
        for(var step=0u;step<4u;step+=1u) {
            let q=vec2<i32>(round(origin+dir*(screen_r*fracs[step])));
            if any(q<lo) || any(q>hi) {continue;}
            let pu=vec2<u32>(q);
            let dist=fg_depth[pu.y*size.x+pu.x];
            if dist<=0.0 {continue;}
            let v=fg_world_at(pu,size,dist)-s.world;
            let d2=dot(v,v);
            if d2>1e-8 && d2<r2 {
                let sin_h=dot(v,s.normal)*inverseSqrt(d2);
                let falloff=1.0-d2/r2;
                let occ=clamp((sin_h-0.05)/0.95,0.0,1.0)*falloff;
                h_max=max(h_max,occ);
            }
        }
        occ_sum+=h_max*h_max;
    }
    return occ_sum*0.125;
}
// One workgroup per FG_BLOCK block, with no shading in it: the pixels without tile probes on
// their plane vote (in an order the frame turns) for the one the block's adaptive probe goes on.
// A block without one leaves its probe invalid. Also records per-pixel linear distance for
// sub-probe corner contact AO (`fg_depth`) at zero extra geometry-decode cost.
var<workgroup> fg_orphan_count:atomic<u32>;
@compute @workgroup_size(8,8) fn fg_classify(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_id) local:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let size=textureDimensions(primary_hit);
    let frame=fg_frame();
    if lane==0u {atomicStore(&fg_orphan,FG_NONE);atomicStore(&fg_orphan_count,0u);}
    workgroupBarrier();
    let q=group.xy*FG_BLOCK+local.xy;
    let mine=fg_surface_at(q,size,false);
    if all(q<size) {
        var dist=0.0;
        if mine.placement!=0u {
            let ray=primary_ray(q,size);
            dist=length(mine.world-ray.near);
        }
        fg_depth[q.y*size.x+q.x]=dist;
    }
    if mine.placement!=0u && fg_tiles(q,size,mine,false)<=0.08 {
        // Pick the covered orphan pixel closest to block center so adaptive probes on curved
        // surfaces and edges keep stable normals across frames instead of resetting history.
        atomicAdd(&fg_orphan_count,1u);
        let dx=i32(local.x)-4;let dy=i32(local.y)-4;
        atomicMin(&fg_orphan,(u32(dx*dx+dy*dy)<<6u)|lane);
    }
    workgroupBarrier();
    if lane!=0u {return;}
    let pick=select(atomicLoad(&fg_orphan),FG_NONE,FG_HALF_RATE && atomicLoad(&fg_orphan_count)<2u);
    let index=fg_adaptive_in(group.xy,size,fg_write_bank());
    let at=fg_at(index,size)+2u;
    let c=gi_prepared[at];
    let stored=select(pick&63u,FG_NONE,pick==FG_NONE);
    gi_prepared[at]=GiPrepared(vec4<u32>(c.r0.xyz,stored),c.r1,c.r2);
    if pick==FG_NONE {fg_invalidate(index,size);}
}
var<workgroup> fg_pair:array<u32,2>;
// Two blocks per workgroup of 64 (fg_classify picked their pixels): lanes 32 b .. 32 b + 31
// trace block b's probe; a pair with no pick exits before any shading.
@compute @workgroup_size(64) fn fg_adapt(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let size=textureDimensions(primary_hit);
    let tiles=(size+vec2<u32>(FG_TILE-1u))/FG_TILE;
    let blocks=(size+vec2<u32>(FG_BLOCK-1u))/FG_BLOCK;
    let frame=fg_frame();
    let b=lane/FG_RAYS;
    let block=vec2<u32>(group.x*2u+b,group.y);
    let inside=all(block<blocks);
    let clamped=min(block,blocks-vec2<u32>(1u));
    let write_idx=fg_adaptive_in(clamped,size,fg_write_bank());
    let state_idx=tiles.x*tiles.y*FG_SLOTS+clamped.y*blocks.x+clamped.x;
    if lane%FG_RAYS==0u {fg_pair[b]=select(FG_NONE,fg_pick_at(write_idx,size),inside);atomicStore(&fg_asked[b],0u);atomicStore(&fg_rays[b],0u);atomicStore(&fg_wg_cold[b],0u);atomicStore(&fg_wg_geom[b],0u);atomicStore(&fg_wg_geom_ans[b],0u);fg_wg_unsh[b]=0.0;}
    let pair=workgroupUniformLoad(&fg_pair);
    if pair[0]==FG_NONE && pair[1]==FG_NONE {return;}
    let pick=pair[b];
    var s:FgSurface;
    var pixel=vec2<u32>(0u);
    if pick!=FG_NONE {
        pixel=block*FG_BLOCK+vec2<u32>(pick%FG_BLOCK,pick/FG_BLOCK);
        s=fg_surface_at(pixel,size,false);
    }
    let footprint=select(0.0,fg_footprint(pixel,size,s.world),s.placement!=0u);
    let prev_idx=select(0xffffffffu,fg_find_prev_adapt(s,size,footprint),s.placement!=0u);
    // Half rate (and eighth rate once truly converged and still, matching fg_trace): align turn
    // parity to the 2-block workgroup (group.x) so both blocks sharing one warp skip together.
    var prev_plane:FgPlane;
    if prev_idx!=0xffffffffu {prev_plane=fg_plane(prev_idx,size);}
    let old_w=prev_plane.point.w;
    let probe_shifted=prev_idx!=0xffffffffu && fg_aniso_dist(s.prev_world-prev_plane.point.xyz,s.normal,s.world,footprint,0.125)>0.05;
    let at=fg_state_at(state_idx);
    let emitter_moved=fg_emitter_moved();
    let adapt_changed=at<arrayLength(&fg_state) && ((fg_state[at]&64u)!=0u || fg_changed_recently(fg_state[at],frame));
    let stable=!fg_recently_changed() && !emitter_moved && !probe_shifted && !adapt_changed && old_w>=28.0 && at<arrayLength(&fg_state) && (fg_state[at]&0x3fu)>=FG_STABLE;
    let turn=select((group.x+block.y+frame)&1u,(group.x+2u*block.y+frame)%FG_STABLE_RATE,stable);
    let moving_direct=probe_shifted && fg_direct_emitters();
    let keep=!emitter_moved && !fg_recently_changed() && !moving_direct && !adapt_changed && FG_HALF_RATE && turn!=0u && s.placement!=0u && old_w>=FG_HALF_MIN;
    var traced=s;
    if keep {traced.placement=0u;}
    if lane%FG_RAYS==0u {fg_trace_mask[b]=u32(traced.placement!=0u);}
    let trace_mask=workgroupUniformLoad(&fg_trace_mask);
    if trace_mask[0]!=0u || trace_mask[1]!=0u {
        fg_lane(lane,lane%FG_RAYS,traced,pixel,fg_hash(state_idx),clamped);
        workgroupBarrier();
        fg_reduce(lane);
        workgroupBarrier();
        fg_count_rays(lane);
    }
    if lane%FG_RAYS!=0u || pick==FG_NONE {return;}
    if s.placement==0u {fg_invalidate(write_idx,size);return;}
    if keep {
        var carried=fg_load(prev_idx,size);
        carried.point=vec4<f32>(s.world,carried.point.w);
        carried.normal=vec4<f32>(s.normal,footprint);
        fg_store(write_idx,size,carried);
        return;
    }
    fg_accumulate(write_idx,prev_idx,state_idx,size,b,s,pixel,false,probe_shifted);
}
@compute @workgroup_size(8,4) fn fg_resolve(@builtin(global_invocation_id) grid:vec3<u32>) {
    let id=grid.xy+primary_region.xy;
    let size=textureDimensions(primary_hit);
    let s=fg_surface(id,size);
    if s.placement==0u {return;}
    let bank=fg_write_bank();
    fg_clear();
    var total=fg_tiles(id,size,s,true);
    if total<=0.08 {
        // Weak or missing tile coverage on this plane: include the 3x3 adaptive probes on this
        // plane with symmetric weights so freshly disoccluded thin ledges, steps, and curved
        // limbs never get overpowered by mature neighbours.
        let blocks=vec2<i32>((size+vec2<u32>(FG_BLOCK-1u))/FG_BLOCK);
        let f=(vec2<f32>(id)+vec2<f32>(0.5))/f32(FG_BLOCK)-vec2<f32>(0.5);
        let own_b=clamp(vec2<i32>(id/FG_BLOCK),vec2<i32>(0),blocks-vec2<i32>(1));
        for(var y=-1;y<=1;y+=1) {
            for(var x=-1;x<=1;x+=1) {
                let b=own_b+vec2<i32>(x,y);
                if any(b<vec2<i32>(0)) || any(b>=blocks) {continue;}
                let index=fg_adaptive_in(vec2<u32>(b),size,bank);
                let p=fg_plane(index,size);
                let d=vec2<f32>(b)-f;
                let w=exp(-dot(d,d)*0.5)*fg_on_plane(p,s.world,s.normal)*min(p.point.w,1.0);
                if w<=1e-6 {continue;}
                fg_add_combined(index,size,w);
                total+=w;
            }
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
                let index=select(fg_filtered(u32(own.y*tiles.x+own.x)*FG_SLOTS+k,size),fg_adaptive_in(id/FG_BLOCK,size,bank),k==FG_SLOTS);
                let p=fg_plane(index,size);
                if p.point.w<=0.0 {continue;}
                let facing=dot(p.normal.xyz,s.normal);
                let w=max(facing,0.0)/(1.0+abs(dot(p.normal.xyz,s.world-p.point.xyz))/max(p.normal.w,1e-6));
                if w>best {best=w;chosen=index;}
            }
        }
        if best>0.0 {fg_add_combined(chosen,size,1.0);}
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
    let occ=fg_corner_ao(id,size,s);
    if occ>1e-4 {
        let sky_ao=clamp(1.0-0.72*occ,0.25,1.0);
        for(var i=0u;i<9u;i+=1u) { gi_rx_sky[i]*=sky_ao; }
        let l0_y=fg_luminance(max(gi_rx_bounce[0],vec3<f32>(0.0)));
        if l0_y>1e-6 {
            let l1_v=vec3<f32>(
                fg_luminance(gi_rx_bounce[3]),
                fg_luminance(gi_rx_bounce[1]),
                fg_luminance(gi_rx_bounce[2]),
            );
            // A uniform isotropic upper hemisphere (white/orange furnace) has l1 == (sqrt(3)/2)*l0*n,
            // so delta_l1 vanishes identically and furnace energy stays exact.
            let delta_l1=l1_v-s.normal*(0.8660254*l0_y);
            let aniso=smoothstep(0.05,0.25,length(delta_l1)/l0_y);
            let bounce_ao=mix(1.0,clamp(1.0-0.62*occ,0.35,1.0),aniso);
            for(var k=0u;k<4u;k+=1u) { gi_rx_bounce[k]*=bounce_ao; }
        }
    }
    gi_rx_valid=true;
    gi_rx_absolute=true;
    gi_rx_normal=s.shading;
    gi_rx_store_at(id.y*size.x+id.x);
}
