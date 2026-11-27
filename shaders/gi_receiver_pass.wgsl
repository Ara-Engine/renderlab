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
// The prepared lookup (gi_rx_*) as pixel `at`'s 48-byte record. Final-gather records
// (gi_rx_absolute) apply a zero-mean +/-0.5 LSB triangular dither before snorm8 packing so
// smooth per-pixel emitter gradients show zero 1/127 quantization contour bands.
fn gi_rx_store_with_dither(at:u32,d:f32) {
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
    let dv=vec4<f32>(d);
    let q0=pack4x8snorm(vec4<f32>(gi_rx_sky[0u].x,gi_rx_sky[0u].y,gi_rx_sky[0u].z,gi_rx_sky[1u].x)/vec4<f32>(ss)+dv);
    let q1=pack4x8snorm(vec4<f32>(gi_rx_sky[1u].y,gi_rx_sky[1u].z,gi_rx_sky[2u].x,gi_rx_sky[2u].y)/vec4<f32>(ss)+dv);
    let q2=pack4x8snorm(vec4<f32>(gi_rx_sky[2u].z,gi_rx_sky[3u].x,gi_rx_sky[3u].y,gi_rx_sky[3u].z)/vec4<f32>(ss)+dv);
    let q3=pack4x8snorm(vec4<f32>(gi_rx_sky[4u].x,gi_rx_sky[4u].y,gi_rx_sky[4u].z,gi_rx_sky[5u].x)/vec4<f32>(ss)+dv);
    let q4=pack4x8snorm(vec4<f32>(gi_rx_sky[5u].y,gi_rx_sky[5u].z,gi_rx_sky[6u].x,gi_rx_sky[6u].y)/vec4<f32>(ss)+dv);
    let q5=pack4x8snorm(vec4<f32>(gi_rx_sky[6u].z,gi_rx_sky[7u].x,gi_rx_sky[7u].y,gi_rx_sky[7u].z)/vec4<f32>(ss)+dv);
    let q6=pack4x8snorm(vec4<f32>(gi_rx_sky[8u].x/ss,gi_rx_sky[8u].y/ss,gi_rx_sky[8u].z/ss,gi_rx_bounce[0u].x/bs)+dv);
    let q7=pack4x8snorm(vec4<f32>(gi_rx_bounce[0u].y,gi_rx_bounce[0u].z,gi_rx_bounce[1u].x,gi_rx_bounce[1u].y)/vec4<f32>(bs)+dv);
    let q8=pack4x8snorm(vec4<f32>(gi_rx_bounce[1u].z,gi_rx_bounce[2u].x,gi_rx_bounce[2u].y,gi_rx_bounce[2u].z)/vec4<f32>(bs)+dv);
    let q9=pack4x8snorm(vec4<f32>(gi_rx_bounce[3u].x/bs+d,gi_rx_bounce[3u].y/bs+d,gi_rx_bounce[3u].z/bs+d,select(0.0,1.0,gi_rx_absolute)));
    gi_prepared[at].r0=vec4<u32>(bitcast<vec2<u32>>(vec2<f32>(select(-1.0,sky,gi_rx_valid),bounce)),q0,q1);
    gi_prepared[at].r1=vec4<u32>(q2,q3,q4,q5);
    gi_prepared[at].r2=vec4<u32>(q6,q7,q8,q9);
}
fn gi_rx_store_at(at:u32) {
    gi_rx_store_with_dither(at,0.0);
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

// ---- final gather + per-pixel direct analytical area emitters (src/gi_receiver_pass.rs) ----
// Indirect GI (sky L2 + multi-bounce L1) runs on screen probes (`fg_trace`, `fg_filter`,
// `fg_filter2`, `fg_classify`, `fg_adapt`) while direct analytical emissive area lights
// (n <= FG_DIRECT_EMITTERS) run at full pixel resolution (`fg_direct_trace` + blocker-distance
// & variance-guided plane-aware à-trous `fg_direct_atrous_1..16`) using a ratio estimator
// (Heitz-Hill-McGuire 2018: U * S_tilde / G_tilde) with exact per-pixel Lambertian F·n,
// tile-classified unanimous early-out, and pre-filter temporal reprojection.
const FG_NON_RT:bool=false;
const FG_TILE:u32=16u;
const FG_SLOTS:u32=2u;
const FG_BLOCK:u32=8u;
const FG_RAYS:u32=32u;
const FG_HISTORY:f32=64.0;
const FG_HALF_RATE:bool=false;
const FG_HALF_MIN:f32=8.0;
const FG_CONE:f32=0.45764912225414734;
const FG_SKY_ALPHA:f32=0.0;
const FG_SEARCH:i32=2;
const FG_PLANE:f32=0.12;
const FG_SH_VECS:u32=10u;
const FG_BOUNCE:u32=28u;
// Pure indirect screen probe: 4 GiPrepared records (192 bytes) — sky L2 (0..26), pad (27),
// indirect bounce L1 (28..39), point (xyz, history count), normal (xyz, tile footprint).
struct FgProbe { sh:array<vec4<f32>,10>, point:vec4<f32>, normal:vec4<f32> };
fn fg_at(index:u32,size:vec2<u32>)->u32 { return size.x*size.y+4u*index; }
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
fn fg_load(index:u32,size:vec2<u32>)->FgProbe {
    let at=fg_at(index,size);
    let a=gi_prepared[at];let b=gi_prepared[at+1u];let c=gi_prepared[at+2u];let e=gi_prepared[at+3u];
    return FgProbe(array<vec4<f32>,10>(bitcast<vec4<f32>>(a.r0),bitcast<vec4<f32>>(a.r1),bitcast<vec4<f32>>(a.r2),
        bitcast<vec4<f32>>(b.r0),bitcast<vec4<f32>>(b.r1),bitcast<vec4<f32>>(b.r2),bitcast<vec4<f32>>(c.r0),
        bitcast<vec4<f32>>(c.r1),bitcast<vec4<f32>>(c.r2),bitcast<vec4<f32>>(e.r0)),
        bitcast<vec4<f32>>(e.r1),bitcast<vec4<f32>>(e.r2));
}
struct FgPlane { point:vec4<f32>, normal:vec4<f32> };
var<private> fg_sum:array<vec4<f32>,10>;
fn fg_clear() {
    for(var v=0u;v<FG_SH_VECS;v+=1u) {fg_sum[v]=vec4<f32>(0.0);}
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

var<workgroup> fg_lane_sh:array<array<f32,40>,64>;
var<workgroup> fg_nee_sh:array<array<f32,12>,64>;
const FG_ANSWERED:u32=27u;

fn fg_hash(x:u32)->u32 {
    var h=x*0x9e3779b1u;h^=h>>16u;h*=0x85ebca6bu;h^=h>>13u;h*=0xc2b2ae35u;h^=h>>16u;return h;
}
// Hierarchical 3-level Owen-XOR Graeco-Latin doubly-stratified 2D blue-noise tile (64x64 period):
// every 4x4 window covers all 16 sub-cells, every row/col covers all 4 strata on u1 and u2,
// level-1/level-2 tile offsets decorrelate atrous step=4/8/16 taps with zero 8px or 16px
// collisions, and 4D Sobol XOR temporal bits keep (u0, u1, u2, u3) pairwise orthogonal so both
// triangles of a 2-tri quad cover all 4 strata equally in the 1-spp path with low EMA edge bias.
fn fg_rand4(id:vec2<u32>,frame:u32)->vec4<f32> {
    let gl=array<u32,16>(0u,14u,9u,7u,11u,5u,2u,12u,13u,3u,4u,10u,6u,8u,15u,1u);
    let p=(id.x&3u)|((id.y&3u)<<2u);
    let g=gl[p];
    let g2=gl[((id.x^2u)&3u)|(((id.y^1u)&3u)<<2u)];
    let tx=id.x>>2u;
    let ty=id.y>>2u;
    let t1=gl[(tx&3u)|((ty&3u)<<2u)];
    let t2=gl[((tx>>2u)&3u)|(((ty>>2u)&3u)<<2u)];
    let vdc16=array<u32,16>(0u,8u,4u,12u,2u,10u,6u,14u,1u,9u,5u,13u,3u,11u,7u,15u);
    let b0=frame&1u;
    let b1=(frame>>1u)&1u;
    let b2=(frame>>2u)&1u;
    let b3=(frame>>3u)&1u;
    let b4=(frame>>4u)&1u;
    let fx=((b1<<1u)|(b2^b4))^(t1&3u)^(t2&3u);
    let fy=(((b0^b1)<<1u)|(b3^b2))^((t1>>2u)&3u)^((t2>>2u)&3u);
    let cx=(g&3u)^fx;
    let cy=((g>>2u)&3u)^fy;
    let ph=fg_hash((id.x*0x9e3779b1u)^(id.y*0x85ebca6bu)^0x68bc21ebu);
    let ph2=fg_hash(ph^0x02e5be93u);
    let ph3=fg_hash(ph2^0xd3a2646cu);
    let ph4=fg_hash(ph3^0x94d049bbu);
    let f=f32(frame&1023u);
    let jx=fract(f32(ph&0xffffu)*(1.0/65536.0)+0.754877666*f);
    let jy=fract(f32(ph2&0xffffu)*(1.0/65536.0)+0.569840296*f);
    let u1=(f32(cx)+jx)*0.25;
    let u2=(f32(cy)+jy)*0.25;
    let o0=(((g2&3u)+(((g2>>2u)&3u)<<2u))^t1^t2)&15u;
    let s_u0=vdc16[frame&15u]^o0;
    let j0=fract(f32(ph3&0xffffu)*(1.0/65536.0)+0.6180339887*f);
    let u0=(f32(s_u0)+j0)*(1.0/16.0);
    // Independent 4th uniform for horizon-clipped quad subtriangle selection (orthogonal to u0/u1/u2).
    let o3=((((g2>>2u)&3u)+((g2&3u)<<2u))^t1^t2)&15u;
    let s_u3=vdc16[((frame^(frame>>2u))^(b4<<3u))&15u]^o3;
    let j3=fract(f32(ph4&0xffffu)*(1.0/65536.0)+0.3819660113*f);
    let u3=(f32(s_u3)+j3)*(1.0/16.0);
    return vec4<f32>(u0,u1,u2,u3);
}
fn fg_oct_encode(n:vec3<f32>)->u32 {
    let l1=max(abs(n.x)+abs(n.y)+abs(n.z),1e-8);
    var p=n.xy/l1;
    if n.z<0.0 { p=(vec2<f32>(1.0)-abs(p.yx))*select(vec2<f32>(-1.0),vec2<f32>(1.0),p>=vec2<f32>(0.0)); }
    let q=vec2<u32>(clamp(round((p*0.5+0.5)*4095.0),vec2<f32>(0.0),vec2<f32>(4095.0)));
    return q.x|(q.y<<12u);
}
fn fg_oct_decode(bits:u32)->vec3<f32> {
    let p=vec2<f32>(f32(bits&4095u),f32((bits>>12u)&4095u))*(2.0/4095.0)-vec2<f32>(1.0);
    var n=vec3<f32>(p.x,p.y,1.0-abs(p.x)-abs(p.y));
    let t=max(-n.z,0.0);
    n.x+=select(t,-t,n.x>=0.0);
    n.y+=select(t,-t,n.y>=0.0);
    return normalize(n);
}

struct FgSurface { world:vec3<f32>, prev_world:vec3<f32>, normal:vec3<f32>, shading:vec3<f32>, epsilon:f32, placement:u32, moved:bool };
fn fg_surface(id:vec2<u32>,size:vec2<u32>)->FgSurface { return fg_surface_at(id,size,true); }
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
fn fg_footprint(pixel:vec2<u32>,size:vec2<u32>,world:vec3<f32>)->f32 {
    let ray=primary_ray(pixel,size);
    let pixel_angle=length(ray.dx_far-ray.dx_near)/max(length(ray.far-ray.near),1e-30);
    return f32(FG_TILE)*pixel_angle*length(world-lighting.eye);
}
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
    let hit=primary_decode(traced.record);
    let emission=gi_triangle_emission(hit.batch,hit.placement,traced.record.y);
    if all(emission<=vec3<f32>(0.0)) {return rl_finite(cell.rgb);}
    fg_unanswered=false;
    let reflected=select(vec3<f32>(0.0),max(cell.rgb-emission,vec3<f32>(0.0)),cell.w>0.0);
    if fg_direct_emitters() {return rl_finite(reflected);}
    let light=fg_light_pdf(fg_luminance(emission),traced.t,abs(dot(unit_normal(rl_cache_geometric),d)));
    return rl_finite(reflected+emission*fg_mis(hemi_pdf*f32(FG_HEMI),light*f32(FG_NEE)));
}
const FG_HEMISPHERE_PDF:f32=0.15915494309189535;
fn fg_luminance(c:vec3<f32>)->f32 { return dot(c,vec3<f32>(0.2126,0.7152,0.0722)); }
fn fg_light_pdf(luminance:f32,t:f32,cosine:f32)->f32 {
    let n=gi_word(GI_H_EMITTER_COUNT);
    if n==0u {return 0.0;}
    let total=bitcast<f32>(gi_word(gi_emitter_cdf_at()+n-1u));
    if !(total>0.0) {return 0.0;}
    return luminance*3.141592653589793*t*t/(total*max(cosine,1e-4));
}
fn fg_mis(own:f32,other:f32)->f32 {
    let a=own*own;
    return a/max(a+other*other,1e-30);
}
var<private> fg_hit:bool;
var<private> fg_unanswered:bool;
var<private> fg_no_value:bool;
var<private> fg_changing:bool;

// Group 2 bindings:
//   30: cache external request list + seen bitset + ray counters
//   31: fg_state (41-word header + 2 banks of per-probe (word, usual) pairs)
//   32: fg_direct_guide (2 banks x 8 B guide + 16 B per-pixel analytical unshadowed U & F_dir)
//   33: fg_direct_data (2 banks x 16 B pre-filter temporal ratio history + 2 x 8 B à-trous ping/pong)
@group(2) @binding(30) var<storage,read_write> fg_requests:array<atomic<u32>>;
@group(2) @binding(31) var<storage,read_write> fg_state:array<u32>;
@group(2) @binding(32) var<storage,read_write> fg_direct_guide:array<vec2<u32>>;
@group(2) @binding(33) var<storage,read_write> fg_direct_data:array<vec2<u32>>;

const FG_REQUESTS_MAX:u32=16384u;
const FG_SEEN:u32=4u+4u*FG_REQUESTS_MAX;
fn fg_touch(slot:u32) {
    if slot==0xffffffffu { return; }
    let w=FG_SEEN+(slot>>5u);
    if w<arrayLength(&fg_requests) { atomicOr(&fg_requests[w],1u<<(slot&31u)); }
}

const FG_STABLE:u32=4u;
const FG_STABLE_RATE:u32=4u;
// Header words in fg_state ahead of the double-buffered per-probe pairs:
//   0: flags (bit 0: recently_changed; bit 1: emitter_moved; bit 2: direct_step16; bit 3: camera_still; bit 4: direct_allowed)
//   1: gather_frame
//   2: bank parity (0 or 1; write_bank = parity, read_bank = 1 - parity)
//   3: has_prev (1 when words 4..40 hold valid previous-frame unjittered camera state)
//   4..20: previous unjittered view-projection mat4x4<f32>
//   20..36: previous jittered inverse view-projection mat4x4<f32>
//   36..39: previous eye xyz
//   39: previous pixel angle (radians)
//   40: GPU-persistent previous estimator mode (0 = none/reset, 1 = direct <=64, 2 = probe NEE)
const FG_STATE_HEAD:u32=41u;
fn fg_recently_changed()->bool { return (fg_state[0]&1u)!=0u; }
fn fg_emitter_moved()->bool { return (fg_state[0]&2u)!=0u; }
fn fg_direct_step16()->bool { return (fg_state[0]&4u)!=0u; }
fn fg_camera_still()->bool { return (fg_state[0]&8u)!=0u; }
fn fg_direct_allowed()->bool { return (fg_state[0]&16u)!=0u; }
fn fg_frame()->u32 { return fg_state[1]; }
fn fg_state_at(index:u32,size:vec2<u32>,bank:u32)->u32 {
    return FG_STATE_HEAD+2u*(bank*fg_bank_stride(size)+index);
}
fn fg_previous_vp()->mat4x4<f32> {
    return mat4x4<f32>(
        vec4<f32>(bitcast<f32>(fg_state[4]),bitcast<f32>(fg_state[5]),bitcast<f32>(fg_state[6]),bitcast<f32>(fg_state[7])),
        vec4<f32>(bitcast<f32>(fg_state[8]),bitcast<f32>(fg_state[9]),bitcast<f32>(fg_state[10]),bitcast<f32>(fg_state[11])),
        vec4<f32>(bitcast<f32>(fg_state[12]),bitcast<f32>(fg_state[13]),bitcast<f32>(fg_state[14]),bitcast<f32>(fg_state[15])),
        vec4<f32>(bitcast<f32>(fg_state[16]),bitcast<f32>(fg_state[17]),bitcast<f32>(fg_state[18]),bitcast<f32>(fg_state[19])),
    );
}
fn fg_previous_inv()->mat4x4<f32> {
    return mat4x4<f32>(
        vec4<f32>(bitcast<f32>(fg_state[20]),bitcast<f32>(fg_state[21]),bitcast<f32>(fg_state[22]),bitcast<f32>(fg_state[23])),
        vec4<f32>(bitcast<f32>(fg_state[24]),bitcast<f32>(fg_state[25]),bitcast<f32>(fg_state[26]),bitcast<f32>(fg_state[27])),
        vec4<f32>(bitcast<f32>(fg_state[28]),bitcast<f32>(fg_state[29]),bitcast<f32>(fg_state[30]),bitcast<f32>(fg_state[31])),
        vec4<f32>(bitcast<f32>(fg_state[32]),bitcast<f32>(fg_state[33]),bitcast<f32>(fg_state[34]),bitcast<f32>(fg_state[35])),
    );
}
fn fg_previous_eye()->vec3<f32> {
    return vec3<f32>(bitcast<f32>(fg_state[36]),bitcast<f32>(fg_state[37]),bitcast<f32>(fg_state[38]));
}
fn fg_previous_pixel_angle()->f32 { return max(bitcast<f32>(fg_state[39]),1e-6); }

// World-position reconstruction from a stored linear ray distance for current and previous frames.
fn fg_world_from_inv(p:vec2<u32>,size:vec2<u32>,dist:f32,inv:mat4x4<f32>)->vec3<f32> {
    let ndc=fma((vec2<f32>(p)+vec2<f32>(0.5))/vec2<f32>(size),vec2<f32>(2.0,-2.0),vec2<f32>(-1.0,1.0));
    let xy=fma(inv[0],vec4<f32>(ndc.x),inv[1]*ndc.y);
    let a=xy+inv[2]+inv[3];
    let b=xy+inv[3];
    let near=a.xyz/a.w;
    let far=b.xyz/b.w;
    return near+normalize(far-near)*dist;
}
fn fg_world_at(p:vec2<u32>,size:vec2<u32>,dist:f32)->vec3<f32> {
    return fg_world_from_inv(p,size,dist,primary_frame.inverse);
}

// Guide & direct buffer accessors.
fn fg_guide_idx(at:u32,n_px:u32,bank:u32)->u32 { return bank*n_px+at; }
fn fg_u_idx(at:u32,n_px:u32)->u32 { return 2u*n_px+2u*at; }
fn fg_hist_idx(at:u32,n_px:u32,bank:u32)->u32 { return (bank*2u)*n_px+2u*at; }
fn fg_ping_idx(at:u32,n_px:u32)->u32 { return 4u*n_px+at; }
fn fg_pong_idx(at:u32,n_px:u32)->u32 { return 5u*n_px+at; }

fn fg_direct_emitters()->bool {
    if !fg_direct_allowed() {return false;}
    let n=gi_word(GI_H_EMITTER_COUNT);
    return n>0u && n<=FG_DIRECT_EMITTERS;
}
const FG_DIRECT_EMITTERS:u32=64u;

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

// ============================================================================================
// Per-pixel tile-classified direct analytical area-emitter pass (n <= FG_DIRECT_EMITTERS)
// ============================================================================================
// Backface culling for closed multi-triangle emitters (boxes/bulbs/cylinders, e.g. e_panel):
// double-sided thin sheets or 1-2 triangle quads emit from both sides, while single-sided
// materials and multi-triangle solids emit only from front-facing triangles (e1,e2 already
// normalized to source-front winding by gi_emitters_build even for mirrored placements).
fn fg_emitter_faces(r:u32,a:vec3<f32>,e1:vec3<f32>,e2:vec3<f32>,q:vec3<f32>)->bool {
    let batch=gi_word(r+12u);
    let place=gi_word(r+13u);
    let tris=batch_triangles(primary_batch_info(batch));
    let flags=primary_placements[place];
    let single_sided=(flags&0x40000000u)!=0u;
    let thin=(flags&0x20000000u)!=0u;
    if !single_sided && (thin || tris<=2u) {return true;}
    return dot(cross(e1,e2),q-a)>0.0;
}

// Exact horizon-clipped uniform sampling on triangle (a, a+e1, a+e2) above receiver plane (q, n).
// Returns (sample_world_pos, clipped_area); clipped_area == 0 when entirely below horizon.
struct FgClipSample { pos:vec3<f32>, area:f32 };
fn fg_sample_clipped_tri(q:vec3<f32>,n:vec3<f32>,a:vec3<f32>,e1:vec3<f32>,e2:vec3<f32>,u:vec3<f32>)->FgClipSample {
    let v0=a-q;let v1=a+e1-q;let v2=a+e2-q;
    let h0=dot(v0,n);let h1=dot(v1,n);let h2=dot(v2,n);
    if h0<=0.0 && h1<=0.0 && h2<=0.0 { return FgClipSample(a,0.0); }
    var p:array<vec3<f32>,4>;
    var m=0u;
    if h0>0.0 { p[m]=v0; m+=1u; }
    if (h0>0.0)!=(h1>0.0) { p[m]=v0+(v1-v0)*(h0/(h0-h1)); m+=1u; }
    if h1>0.0 { p[m]=v1; m+=1u; }
    if (h1>0.0)!=(h2>0.0) { p[m]=v1+(v2-v1)*(h1/(h1-h2)); m+=1u; }
    if h2>0.0 { p[m]=v2; m+=1u; }
    if (h2>0.0)!=(h0>0.0) { p[m]=v2+(v0-v2)*(h2/(h2-h0)); m+=1u; }
    if m<3u { return FgClipSample(a,0.0); }
    let a0=0.5*length(cross(p[1]-p[0],p[2]-p[0]));
    var a1=0.0;
    if m==4u { a1=0.5*length(cross(p[2]-p[0],p[3]-p[0])); }
    let total=a0+a1;
    if total<=1e-10 { return FgClipSample(a,0.0); }
    var r1=u.x;var r2=u.y;
    if r1+r2>1.0 { r1=1.0-r1; r2=1.0-r2; }
    var rel=p[0]+(p[1]-p[0])*r1+(p[2]-p[0])*r2;
    if m==4u && u.z*total>a0 {
        rel=p[0]+(p[2]-p[0])*r1+(p[3]-p[0])*r2;
    }
    return FgClipSample(q+rel,total);
}

struct FgDirectHist { s:vec3<f32>, g:vec3<f32>, n:f32, pen:f32 };
fn fg_load_direct_hist(at:u32,n_px:u32,bank:u32)->FgDirectHist {
    let idx=fg_hist_idx(at,n_px,bank);
    let w0=fg_direct_data[idx];
    let w1=fg_direct_data[idx+1u];
    let sg=unpack2x16float(w0.x);
    let bg=unpack2x16float(w0.y);
    let gb=unpack2x16float(w1.x);
    let np=unpack2x16float(w1.y);
    return FgDirectHist(vec3<f32>(sg.x,sg.y,bg.x),vec3<f32>(bg.y,gb.x,gb.y),np.x,np.y);
}
fn fg_store_direct_hist(at:u32,n_px:u32,bank:u32,h:FgDirectHist) {
    let idx=fg_hist_idx(at,n_px,bank);
    let s=clamp(h.s,vec3<f32>(0.0),vec3<f32>(65000.0));
    let g=clamp(h.g,vec3<f32>(0.0),vec3<f32>(65000.0));
    fg_direct_data[idx]=vec2<u32>(pack2x16float(vec2<f32>(s.x,s.y)),pack2x16float(vec2<f32>(s.z,g.x)));
    fg_direct_data[idx+1u]=vec2<u32>(pack2x16float(vec2<f32>(g.y,g.z)),pack2x16float(vec2<f32>(clamp(h.n,0.0,64.0),clamp(h.pen,-1.0,200.0))));
}

var<workgroup> fg_wg_direct_on:u32;
var<workgroup> fg_wg_tile_u:u32;
var<workgroup> fg_wg_tile_flags:atomic<u32>;
var<workgroup> fg_wg_probe_votes:atomic<u32>;
var<workgroup> fg_wg_ref_lane:atomic<u32>;
var<workgroup> fg_wg_ref_p:vec3<f32>;
var<workgroup> fg_wg_ref_n:vec3<f32>;
var<workgroup> fg_wg_dir_rays:atomic<u32>;
var<workgroup> fg_wg_raw_v:array<f32,32>;
var<workgroup> fg_wg_raw_p:array<vec3<f32>,32>;
var<workgroup> fg_wg_raw_n:array<vec3<f32>,32>;
var<workgroup> fg_wg_pen:array<f32,32>;

// Merged per-pixel guide write + exact analytical unshadowed U + pre-filter temporal reprojection
// + 8x4 tile-classified adaptive shadow ray trace + workgroup anti-ghosting clamp + temporal blend.
@compute @workgroup_size(8,4) fn fg_direct_trace(
    @builtin(workgroup_id) group:vec3<u32>,
    @builtin(local_invocation_id) local:vec3<u32>,
    @builtin(local_invocation_index) lane:u32,
) {
    if lane==0u {
        fg_wg_direct_on=u32(fg_direct_emitters());
        atomicStore(&fg_wg_tile_flags,0u);
        atomicStore(&fg_wg_probe_votes,0u);
        atomicStore(&fg_wg_ref_lane,32u);
        atomicStore(&fg_wg_dir_rays,0u);
    }
    if workgroupUniformLoad(&fg_wg_direct_on)==0u {return;}

    let size=textureDimensions(primary_hit);
    let n_px=size.x*size.y;
    let id=group.xy*vec2<u32>(8u,4u)+local.xy;
    let in_bounds=all(id<size);
    let at=select(0u,id.y*size.x+id.x,in_bounds);
    let wbank=fg_write_bank();
    let rbank=fg_read_bank();
    let s=fg_surface_at(id,size,true);
    let cur_ph=select(0u,((s.placement*0x9e3779b1u)>>24u)&0xffu,s.placement!=0u);
    if in_bounds {
        var dist=0.0;
        var norm_tag=0u;
        if s.placement!=0u {
            let ray=primary_ray(id,size);
            dist=length(s.world-ray.near);
            norm_tag=fg_oct_encode(s.normal)|(cur_ph<<24u);
        }
        fg_direct_guide[fg_guide_idx(at,n_px,wbank)]=vec2<u32>(bitcast<u32>(dist),norm_tag);
    }

    let frame=fg_frame();
    let rnd=fg_rand4(id,frame);
    let u0=rnd.x;
    let u1=rnd.y;
    let u2=rnd.z;
    let u3=rnd.w;

    // 1) Exact per-pixel analytical unshadowed Lambertian irradiance U = sum(L_e * max(F_e · n_sh, 0))
    //    using smooth shading normal `s.shading` (no flat-faceted cylinders) and 2-slot stratified
    //    weighted reservoir selection (`u0`, `fract(u0 + 0.5)`) so multi-spp pixels cover both
    //    halves of quad/multi-triangle emitters cleanly instead of locking all rays to one triangle.
    var U=vec3<f32>(0.0);
    var F_dir_sum=vec3<f32>(0.0);
    var W=0.0;
    var chosen_e=0xffffffffu;
    var chosen_irrad=0.0;
    var chosen_rad=vec3<f32>(0.0);
    var chosen_e2=0xffffffffu;
    var chosen_irrad2=0.0;
    var chosen_rad2=vec3<f32>(0.0);
    var u_res=u0;
    var u_res2=fract(u0+0.5);
    let n_emit=gi_word(GI_H_EMITTER_COUNT);
    let base=gi_word(GI_H_EMITTERS);
    if s.placement!=0u {
        for(var e=0u;e<n_emit;e+=1u) {
            let r=base+e*GI_EMITTER_WORDS;
            let rad=gi_emitter_vec(r+9u);
            if all(rad<=vec3<f32>(0.0)) {continue;}
            let a=gi_emitter_vec(r);
            let e1=gi_emitter_vec(r+3u);
            let e2=gi_emitter_vec(r+6u);
            if !fg_emitter_faces(r,a,e1,e2,s.world) {continue;}
            let sh=gi_emitter_sh(s.world,s.shading,a,a+e1,a+e2);
            if sh.x<=2.8e-7 {continue;}
            let f_vec=vec3<f32>(sh.w,sh.y,sh.z)*(1.0/0.488603);
            let irrad=max(dot(f_vec,s.shading),0.0);
            if irrad<=1e-7 {continue;}
            let lum=fg_luminance(rad);
            let w_e=lum*irrad;
            U+=rad*irrad;
            F_dir_sum+=f_vec*lum;
            W+=w_e;
            let p=w_e/max(W,1e-30);
            if u_res<p {
                chosen_e=e;
                chosen_irrad=irrad;
                chosen_rad=rad;
                u_res=u_res/max(p,1e-7);
            } else {
                u_res=(u_res-p)/max(1.0-p,1e-7);
            }
            if u_res2<p {
                chosen_e2=e;
                chosen_irrad2=irrad;
                chosen_rad2=rad;
                u_res2=u_res2/max(p,1e-7);
            } else {
                u_res2=(u_res2-p)/max(1.0-p,1e-7);
            }
        }
    }
    let lit_need=s.placement!=0u && W>1e-7 && fg_luminance(U)*lighting.exposure>=4e-4;
    if in_bounds {
        let f_unit=select(s.shading,unit_normal(F_dir_sum),dot(F_dir_sum,F_dir_sum)>1e-12);
        let u_idx=fg_u_idx(at,n_px);
        fg_direct_guide[u_idx]=vec2<u32>(bitcast<u32>(U.x),bitcast<u32>(U.y));
        fg_direct_guide[u_idx+1u]=vec2<u32>(bitcast<u32>(U.z),fg_oct_encode(f_unit)|(select(0u,1u,lit_need)<<24u));
    }

    // 2) Pre-filter temporal reprojection from previous bank (with exact jittered prev_inv).
    //    Invalidated whenever the previous frame did not run the direct pass (fg_state[40] != 1u).
    var hist=FgDirectHist(vec3<f32>(0.0),vec3<f32>(0.0),0.0,-1.0);
    var re_blur=0.0;
    let emitter_moved=fg_emitter_moved();
    let prev_pa=fg_previous_pixel_angle();
    let world_per_px=select(0.01,prev_pa*max(length(s.world-lighting.eye),0.05),s.placement!=0u);
    if lit_need && !emitter_moved && fg_state[3]!=0u && fg_state[40u]==1u {
        let prev_inv=fg_previous_inv();
        let prev_eye=fg_previous_eye();
        let prev_wpp=prev_pa*max(length(s.prev_world-prev_eye),0.05);
        let max_perr=max(2.5*prev_wpp,4.0*s.epsilon);
        if fg_camera_still() && !s.moved {
            let gword=fg_direct_guide[fg_guide_idx(at,n_px,rbank)];
            let t_dist=bitcast<f32>(gword.x);
            if t_dist>0.0 && (gword.y>>24u)==cur_ph {
                let t_n=fg_oct_decode(gword.y);
                let t_p=fg_world_from_inv(id,size,t_dist,prev_inv);
                let dp=t_p-s.prev_world;
                let perr=max(abs(dot(s.normal,dp)),abs(dot(t_n,dp)));
                if dot(t_n,s.normal)>=0.94 && perr<=max_perr {
                    hist=fg_load_direct_hist(at,n_px,rbank);
                }
            }
        } else {
            let rp=fg_reproject_pixel(s.prev_world,size);
            if rp.z>0.0 {
                let f=rp.xy-vec2<f32>(0.5);
                let base_p=vec2<i32>(floor(f));
                let fr=f-floor(f);
                re_blur=fr.x*(1.0-fr.x)+fr.y*(1.0-fr.y);
                var w_sum=0.0;
                var s_sum=vec3<f32>(0.0);
                var g_sum=vec3<f32>(0.0);
                var n_sum=0.0;
                var pen_sum=0.0;
                var pen_w=0.0;
                for(var k=0u;k<4u;k+=1u) {
                    let o=vec2<i32>(i32(k&1u),i32(k>>1u));
                    let tp=base_p+o;
                    if any(tp<vec2<i32>(0)) || any(tp>=vec2<i32>(size)) {continue;}
                    let bw=select(1.0-fr.x,fr.x,o.x!=0)*select(1.0-fr.y,fr.y,o.y!=0);
                    if bw<=1e-4 {continue;}
                    let t_at=u32(tp.y)*size.x+u32(tp.x);
                    let gword=fg_direct_guide[fg_guide_idx(t_at,n_px,rbank)];
                    let t_dist=bitcast<f32>(gword.x);
                    if t_dist<=0.0 || (gword.y>>24u)!=cur_ph {continue;}
                    let t_n=fg_oct_decode(gword.y);
                    if dot(t_n,s.normal)<0.94 {continue;}
                    let t_p=fg_world_from_inv(vec2<u32>(tp),size,t_dist,prev_inv);
                    let dp=t_p-s.prev_world;
                    let perr=max(abs(dot(s.normal,dp)),abs(dot(t_n,dp)));
                    if perr>max_perr {continue;}
                    let htap=fg_load_direct_hist(t_at,n_px,rbank);
                    if htap.n<=0.0 {continue;}
                    w_sum+=bw;
                    s_sum+=htap.s*bw;
                    g_sum+=htap.g*bw;
                    n_sum+=htap.n*bw;
                    if htap.pen>=0.0 {pen_sum+=htap.pen*bw;pen_w+=bw;}
                }
                if w_sum>=0.25 {
                    let inv_w=1.0/w_sum;
                    hist.s=s_sum*inv_w;
                    hist.g=g_sum*inv_w;
                    hist.n=n_sum*inv_w;
                    hist.pen=select(-1.0,pen_sum/max(pen_w,1e-6),pen_w>1e-4);
                }
            }
        }
    }

    // 3) 8x4 tile unanimity classifier: warm planar interiors with unanimous lit/dark history
    //    trace only 4 rotating stratified probe rays (8x fewer rays); any disagreement, penumbra,
    //    disocclusion, or edge traces full 1x1 pixels cleanly.
    if lit_need { atomicMin(&fg_wg_ref_lane,lane); }
    workgroupBarrier();
    let ref_lane=atomicLoad(&fg_wg_ref_lane);
    if lane==ref_lane {
        fg_wg_ref_p=s.world;
        fg_wg_ref_n=s.normal;
    }
    workgroupBarrier();
    if lit_need {
        atomicOr(&fg_wg_tile_flags,1u);
        let h_r=select(vec3<f32>(0.5),hist.s/max(hist.g,vec3<f32>(1e-5)),hist.n>0.0 && any(hist.g>vec3<f32>(1e-5)));
        let h_y=fg_luminance(h_r);
        let warm_lit=hist.n>=6.0 && h_y>=0.995 && hist.pen<0.0;
        let warm_dark=hist.n>=6.0 && h_y<=0.005;
        if !warm_lit { atomicOr(&fg_wg_tile_flags,2u); }
        if !warm_dark { atomicOr(&fg_wg_tile_flags,4u); }
        let d_ref=s.world-fg_wg_ref_p;
        let p_ref=max(abs(dot(s.normal,d_ref)),abs(dot(fg_wg_ref_n,d_ref)));
        if s.moved || dot(s.normal,fg_wg_ref_n)<0.97 || p_ref>3.0*world_per_px {
            atomicOr(&fg_wg_tile_flags,8u);
        }
        if emitter_moved {
            atomicOr(&fg_wg_tile_flags,16u);
        }
    }
    workgroupBarrier();
    let tflags=atomicLoad(&fg_wg_tile_flags);
    let can_probe=(tflags&1u)!=0u && (tflags&24u)==0u && ((tflags&2u)==0u || (tflags&4u)==0u);
    let expected_lit=(tflags&2u)==0u;

    // 4) Adaptive shadow ray tracing (returning blocker distance t_hit for penumbra width).
    let probe_slot=(frame+group.x*3u+group.y*5u)&7u;
    let is_probe=((local.x&3u)==(probe_slot&3u)) && ((local.y&1u)==(probe_slot>>2u));
    var s_raw=vec3<f32>(0.0);
    var g_raw=vec3<f32>(0.0);
    var pen_raw=-1.0;
    var traced_spp=0u;
    let origin=fma(s.normal,vec3<f32>(s.epsilon),s.world);

    // Phase A: unanimous tile 4-probe verification (bit 1 = traced, bit 0 = disagreed).
    if can_probe && lit_need && is_probe && chosen_e!=0xffffffffu {
        let r=base+chosen_e*GI_EMITTER_WORDS;
        let a=gi_emitter_vec(r);
        let e1=gi_emitter_vec(r+3u);
        let e2=gi_emitter_vec(r+6u);
        let cs=fg_sample_clipped_tri(s.world,s.shading,a,e1,e2,vec3<f32>(u1,u2,u3));
        if cs.area>0.0 {
            let to=cs.pos-origin;
            let dist=length(to);
            let tmax=dist-max(s.epsilon*4.0,2e-3);
            let d=to/max(dist,1e-6);
            var blocked=dot(d,s.normal)<=0.0;
            var t_hit=-1.0;
            if !blocked && tmax>s.epsilon {
                atomicAdd(&fg_wg_dir_rays,1u);
                t_hit=rl_shadow_hit_t(origin,d,s.epsilon,tmax);
                blocked=t_hit>=0.0;
            }
            let cos_r=max(dot(d,s.shading),0.0);
            let cos_e=abs(dot(unit_normal(cross(e1,e2)),d));
            let g_fac=cos_r*cos_e/max(dist*dist,1e-8);
            let rho=clamp(cs.area*g_fac/max(chosen_irrad,1e-8),0.0,8.0);
            let chi=chosen_rad*(W/max(fg_luminance(chosen_rad),1e-8))/max(U,vec3<f32>(1e-8));
            let gw=max(rho*chi,vec3<f32>(1e-5));
            g_raw+=gw;
            if !blocked { s_raw+=gw; }
            else if t_hit>=0.0 {
                let diam=max(length(e1),max(length(e2),length(e2-e1)));
                pen_raw=diam*t_hit/max(dist-t_hit,0.05);
            }
            traced_spp=1u;
            atomicOr(&fg_wg_probe_votes,2u);
            if blocked==expected_lit { atomicOr(&fg_wg_probe_votes,1u); }
        }
    }
    workgroupBarrier();
    if lane==0u { fg_wg_tile_u=u32(can_probe && atomicLoad(&fg_wg_probe_votes)==2u); }
    let tile_unanimous=workgroupUniformLoad(&fg_wg_tile_u)!=0u;

    // Phase B: full 1x1 tracing for penumbra/edge/disoccluded tiles (or tiles whose probe disagreed).
    // Two-stage cold early-out (moving emitters only): trace sample k=0 across the 8x4 workgroup
    // first; if all 32/32 participating lanes on a planar tile unanimously agree (all lit or all
    // blocked across stratified emitter strata), skip redundant k=1..3 rays while partial, edge,
    // and penumbra tiles still run the full 4-spp budget.
    if !tile_unanimous {
        if can_probe && lit_need && chosen_e!=0xffffffffu { hist.n=min(hist.n,1.0); }
        let active_b=lit_need && chosen_e!=0xffffffffu && traced_spp==0u;
        let moving_soft=!fg_camera_still() && (hist.n<6.0 || (hist.pen>=0.25 && hist.n<12.0));
        let cut_settle=fg_camera_still() && fg_direct_step16() && !emitter_moved && hist.n<5.0;
        let spp=select(
            select(select(select(1u,2u,moving_soft),2u,hist.n<4.0),4u,(hist.n<1.0 && !emitter_moved) || cut_settle),
            select(select(select(1u,2u,moving_soft),3u,hist.n<4.0),4u,hist.n<1.0 || cut_settle),
            !FG_NON_RT,
        );
        let k_off=array<vec3<f32>,4>(
            vec3<f32>(0.0,0.0,0.0),
            vec3<f32>(0.25,0.75,0.25),
            vec3<f32>(0.5,0.5,0.5),
            vec3<f32>(0.75,0.25,0.75),
        );
        if active_b {
            let r=base+chosen_e*GI_EMITTER_WORDS;
            let a=gi_emitter_vec(r);
            let e1=gi_emitter_vec(r+3u);
            let e2=gi_emitter_vec(r+6u);
            let tri_n=unit_normal(cross(e1,e2));
            let diam=max(length(e1),max(length(e2),length(e2-e1)));
            let chi=chosen_rad*(W/max(fg_luminance(chosen_rad),1e-8))/max(U,vec3<f32>(1e-8));
            let cs=fg_sample_clipped_tri(s.world,s.shading,a,e1,e2,vec3<f32>(u1,u2,u3));
            if cs.area>0.0 {
                let to=cs.pos-origin;
                let dist=length(to);
                let tmax=dist-max(s.epsilon*4.0,2e-3);
                let d=to/max(dist,1e-6);
                var blocked=dot(d,s.normal)<=0.0;
                var t_hit=-1.0;
                if !blocked && tmax>s.epsilon {
                    atomicAdd(&fg_wg_dir_rays,1u);
                    t_hit=rl_shadow_hit_t(origin,d,s.epsilon,tmax);
                    blocked=t_hit>=0.0;
                }
                let cos_r=max(dot(d,s.shading),0.0);
                let cos_e=abs(dot(tri_n,d));
                let g_fac=cos_r*cos_e/max(dist*dist,1e-8);
                let rho=clamp(cs.area*g_fac/max(chosen_irrad,1e-8),0.0,8.0);
                let gw=max(rho*chi,vec3<f32>(1e-5));
                g_raw+=gw;
                if !blocked { s_raw+=gw; }
                else if t_hit>=0.0 {
                    let wp=diam*t_hit/max(dist-t_hit,0.05);
                    pen_raw=select(min(pen_raw,wp),wp,pen_raw<0.0);
                }
                traced_spp+=1u;
                atomicOr(&fg_wg_probe_votes,select(4u,8u,blocked));
                atomicAdd(&fg_wg_probe_votes,16u);
            }
        }
        workgroupBarrier();
        let k0_raw=atomicLoad(&fg_wg_probe_votes);
        let k0_votes=k0_raw&12u;
        let k0_unanimous=(tflags&8u)==0u && (k0_raw>>4u)==32u && (k0_votes==4u || k0_votes==8u);
        let skip_rest=k0_unanimous && emitter_moved;
        if active_b && !skip_rest {
            for(var k=1u;k<spp;k+=1u) {
                let use2=(k&1u)!=0u && chosen_e2!=0xffffffffu;
                let e_idx=select(chosen_e,chosen_e2,use2);
                let e_irrad=select(chosen_irrad,chosen_irrad2,use2);
                let e_rad=select(chosen_rad,chosen_rad2,use2);
                let r=base+e_idx*GI_EMITTER_WORDS;
                let a=gi_emitter_vec(r);
                let e1=gi_emitter_vec(r+3u);
                let e2=gi_emitter_vec(r+6u);
                let tri_n=unit_normal(cross(e1,e2));
                let diam=max(length(e1),max(length(e2),length(e2-e1)));
                let chi=e_rad*(W/max(fg_luminance(e_rad),1e-8))/max(U,vec3<f32>(1e-8));
                let uu=fract(vec3<f32>(u1,u2,u3)+k_off[k]);
                let cs=fg_sample_clipped_tri(s.world,s.shading,a,e1,e2,uu);
                if cs.area<=0.0 {continue;}
                let to=cs.pos-origin;
                let dist=length(to);
                let tmax=dist-max(s.epsilon*4.0,2e-3);
                let d=to/max(dist,1e-6);
                var blocked=dot(d,s.normal)<=0.0;
                var t_hit=-1.0;
                if !blocked && tmax>s.epsilon {
                    atomicAdd(&fg_wg_dir_rays,1u);
                    t_hit=rl_shadow_hit_t(origin,d,s.epsilon,tmax);
                    blocked=t_hit>=0.0;
                }
                let cos_r=max(dot(d,s.shading),0.0);
                let cos_e=abs(dot(tri_n,d));
                let g_fac=cos_r*cos_e/max(dist*dist,1e-8);
                let rho=clamp(cs.area*g_fac/max(e_irrad,1e-8),0.0,8.0);
                let gw=max(rho*chi,vec3<f32>(1e-5));
                g_raw+=gw;
                if !blocked { s_raw+=gw; }
                else if t_hit>=0.0 {
                    let wp=diam*t_hit/max(dist-t_hit,0.05);
                    pen_raw=select(min(pen_raw,wp),wp,pen_raw<0.0);
                }
                traced_spp+=1u;
            }
        }
        if active_b && traced_spp>0u {
            let inv_spp=1.0/f32(traced_spp);
            s_raw*=inv_spp;
            g_raw*=inv_spp;
        }
    }

    // 5) Workgroup-local same-plane anti-ghosting moment clamp & 3x3 penumbra dilation (zero race!).
    fg_wg_raw_v[lane]=select(-1.0,fg_luminance(s_raw/max(g_raw,vec3<f32>(1e-5))),traced_spp>0u);
    fg_wg_raw_p[lane]=s.world;
    fg_wg_raw_n[lane]=s.normal;
    fg_wg_pen[lane]=select(pen_raw,hist.pen,pen_raw<0.0);
    workgroupBarrier();

    if lane==0u {
        let dr=atomicLoad(&fg_wg_dir_rays);
        if dr>0u { atomicAdd(&fg_requests[3u],dr); }
    }

    if !in_bounds {return;}
    if !lit_need {
        fg_store_direct_hist(at,n_px,wbank,FgDirectHist(vec3<f32>(0.0),vec3<f32>(0.0),0.0,-1.0));
        return;
    }
    let max_direct_n=select(24.0,64.0,fg_camera_still() && !s.moved);
    if tile_unanimous {
        let uv=select(0.0,1.0,expected_lit);
        fg_store_direct_hist(at,n_px,wbank,FgDirectHist(vec3<f32>(uv),vec3<f32>(1.0),min(hist.n+1.0,max_direct_n),-1.0));
        return;
    }

    // Dilation of blocker penumbra width into adjacent lit pixels within the 8x4 tile,
    // plus same-plane local raw visibility mean R_s for anti-ghosting on moving casters.
    var sum_v=0.0;
    var cnt_v=0.0;
    var dil_pen=fg_wg_pen[lane];
    let lx=i32(local.x);let ly=i32(local.y);
    for(var dy=-1;dy<=1;dy+=1) {
        let ny=ly+dy;
        if ny<0 || ny>=4 {continue;}
        for(var dx=-2;dx<=2;dx+=1) {
            let nx=lx+dx;
            if nx<0 || nx>=8 {continue;}
            let j=u32(ny*8+nx);
            let jn=fg_wg_raw_n[j];
            if dot(jn,s.normal)<0.94 {continue;}
            let dp=fg_wg_raw_p[j]-s.world;
            if max(abs(dot(s.normal,dp)),abs(dot(jn,dp)))>max(2.5*world_per_px,4.0*s.epsilon) {continue;}
            let jv=fg_wg_raw_v[j];
            if jv>=0.0 { sum_v+=jv; cnt_v+=1.0; }
            let jp=fg_wg_pen[j];
            if jp>=0.0 && abs(dx)<=1 {
                dil_pen=select(min(dil_pen,jp),jp,dil_pen<0.0);
            }
        }
    }

    // Temporal blend (non-feedback: reads and writes unblurred H_buf).
    var out_h=hist;
    if traced_spp>0u {
        if emitter_moved { hist.n=0.0; }
        if s.moved { hist.n=min(hist.n,2.0); }
        let pen_use=select(dil_pen,pen_raw,pen_raw>=0.0);
        let pen_px=select(8.0,clamp(pen_use/max(world_per_px,1e-5),0.5,32.0),pen_use>=0.0);
        // Penumbra-scaled blur budget: sharp contact shadows keep ~2-4 frames in motion
        // (zero smear), while wide soft penumbrae and unshadowed lit interiors keep up to 24 frames.
        let n_cap=select(24.0,clamp(1.0+(0.35+0.35*pen_px*pen_px)/max(re_blur,1e-4),1.0,24.0),re_blur>1e-5 && !fg_camera_still());
        hist.n=min(hist.n,n_cap);
        // Anti-ghosting moment clamp against same-plane raw visibility R_s.
        if hist.n>0.0 && cnt_v>=6.0 {
            let r_s=sum_v/cnt_v;
            let sig=sqrt(clamp(r_s,0.0,1.0)*(1.0-clamp(r_s,0.0,1.0))/cnt_v+1.0/(cnt_v*cnt_v));
            let h_r=fg_luminance(hist.s/max(hist.g,vec3<f32>(1e-5)));
            let k_clamp=select(3.5,4.5,fg_camera_still() && !s.moved);
            let clamped=clamp(h_r,max(r_s-k_clamp*sig,0.0),min(r_s+k_clamp*sig,1.0));
            if abs(h_r-clamped)>select(0.10,0.35,fg_camera_still() && !s.moved) {
                let scale=select(0.0,clamped/max(h_r,1e-5),h_r>1e-5);
                hist.s=select(hist.g*clamped,hist.s*scale,h_r>1e-5);
                hist.n=min(hist.n,2.0);
            }
        }
        let next_n=min(hist.n+f32(traced_spp),max_direct_n);
        let alpha=f32(traced_spp)/max(next_n,1.0);
        out_h.s=select(s_raw,mix(hist.s,s_raw,alpha),hist.n>0.0);
        out_h.g=select(g_raw,mix(hist.g,g_raw,alpha),hist.n>0.0);
        out_h.n=next_n;
        if pen_raw>=0.0 {
            out_h.pen=select(pen_raw,mix(hist.pen,pen_raw,max(alpha,0.25)),hist.pen>=0.0);
        } else if hist.pen>=0.0 {
            out_h.pen=hist.pen;
        } else {
            out_h.pen=dil_pen;
        }
    } else if out_h.pen<0.0 {
        out_h.pen=dil_pen;
    }
    fg_store_direct_hist(at,n_px,wbank,out_h);
}

// Blocker-distance & variance-guided plane-aware à-trous spatial denoiser on ratio R = S / G.
fn fg_load_atrous_tap(idx:u32)->vec4<f32> {
    let w=fg_direct_data[idx];
    let a=unpack2x16float(w.x);
    let b=unpack2x16float(w.y);
    return vec4<f32>(a.x,a.y,b.x,b.y);
}
fn fg_store_atrous_tap(idx:u32,rv:vec4<f32>) {
    fg_direct_data[idx]=vec2<u32>(pack2x16float(vec2<f32>(rv.x,rv.y)),pack2x16float(vec2<f32>(rv.z,rv.w)));
}

fn fg_direct_atrous_pass(id:vec2<u32>,step:i32,pass_idx:u32) {
    if !fg_direct_emitters() {return;}
    let size=textureDimensions(primary_hit);
    if any(id>=size) {return;}
    let n_px=size.x*size.y;
    let at=id.y*size.x+id.x;
    let wbank=fg_write_bank();
    let dst_idx=select(fg_pong_idx(at,n_px),fg_ping_idx(at,n_px),(pass_idx&1u)==0u);
    let g_c=fg_direct_guide[fg_guide_idx(at,n_px,wbank)];
    let dist_c=bitcast<f32>(g_c.x);
    let u_flag=fg_direct_guide[fg_u_idx(at,n_px)+1u].y>>24u;
    if dist_c<=0.0 || u_flag==0u {
        fg_store_atrous_tap(dst_idx,vec4<f32>(0.0));
        return;
    }
    let h_c=fg_load_direct_hist(at,n_px,wbank);
    var r_c=vec3<f32>(0.0);
    var var_c=0.0;
    if pass_idx==0u {
        r_c=select(vec3<f32>(0.0),clamp(h_c.s/max(h_c.g,vec3<f32>(1e-6)),vec3<f32>(0.0),vec3<f32>(4.0)),any(h_c.s>vec3<f32>(0.0)) && any(h_c.g>vec3<f32>(1e-6)));
    } else {
        let src_idx=select(fg_pong_idx(at,n_px),fg_ping_idx(at,n_px),(pass_idx&1u)==1u);
        let rv=fg_load_atrous_tap(src_idx);
        r_c=rv.xyz;
        var_c=rv.w;
    }
    let n_c=fg_oct_decode(g_c.y);
    let p_c=fg_world_at(id,size,dist_c);
    let world_per_px=fg_previous_pixel_angle()*max(length(p_c-lighting.eye),0.05);
    let pen_c=h_c.pen;

    let hist_t=smoothstep(3.0,18.0,h_c.n);
    let cold_cut=fg_direct_step16() && h_c.n<=8.5 && (fg_camera_still() || fg_state[3]==0u);
    // Fast path for warm zero-variance unanimous pixels (lit or dark), sharp contact shadows
    // (whose narrow penumbra stops wide steps), and step-16 wide-penumbra guard.
    if pass_idx>0u {
        let y_c=fg_luminance(r_c);
        if var_c<1e-5 && h_c.n>=8.0 && (y_c<=1e-4 || y_c>=0.9995) {
            fg_store_atrous_tap(dst_idx,vec4<f32>(r_c,var_c));
            return;
        }
        if pass_idx==4u && (h_c.n>=10.0 || (!cold_cut && var_c<0.01)) {
            let pen_px_16=select(select(0.0,20.0,y_c>0.005 && y_c<0.995 && h_c.n<16.0),clamp(pen_c/max(world_per_px,1e-5),0.5,48.0),pen_c>=0.0);
            if pen_px_16<12.0 || (pen_px_16<16.0 && h_c.n>=28.0) {
                fg_store_atrous_tap(dst_idx,vec4<f32>(r_c,var_c));
                return;
            }
        }
        // Only apply narrow-penumbra early-out once history is warm (h_c.n >= 10), so cold cut and
        // moving-emitter frames filter hit and missed neighbors uniformly without dark speckling.
        if pen_c>=0.0 && h_c.n>=10.0 {
            let pen_px=clamp(pen_c/max(world_per_px,1e-5),0.5,48.0);
            let pen_scale=mix(1.4,0.72,hist_t);
            if f32(step)>pen_px*pen_scale {
                fg_store_atrous_tap(dst_idx,vec4<f32>(r_c,var_c));
                return;
            }
        }
    }

    let plane_tol=max(2.5*world_per_px*clamp(f32(step)*0.5,1.0,4.0),2e-3);
    // Pass 0: compute same-plane 3x3 spatial variance + Bernoulli variance floor for cold whip-pan
    // strips (where binary samples can have zero 3x3 variance in penumbra tails) and gather min penumbra.
    var pen_eff=pen_c;
    if pass_idx==0u {
        var m1=0.0;
        var m2=0.0;
        var mn=0.0;
        for(var dy=-1;dy<=1;dy+=1) {
            for(var dx=-1;dx<=1;dx+=1) {
                let q=vec2<i32>(id)+vec2<i32>(dx,dy);
                if any(q<vec2<i32>(0)) || any(q>=vec2<i32>(size)) {continue;}
                let q_at=u32(q.y)*size.x+u32(q.x);
                let g_q=fg_direct_guide[fg_guide_idx(q_at,n_px,wbank)];
                let d_q=bitcast<f32>(g_q.x);
                if d_q<=0.0 {continue;}
                let n_q=fg_oct_decode(g_q.y);
                if dot(n_c,n_q)<0.94 {continue;}
                let dp=fg_world_at(vec2<u32>(q),size,d_q)-p_c;
                if max(abs(dot(n_c,dp)),abs(dot(n_q,dp)))>plane_tol {continue;}
                let h_q=fg_load_direct_hist(q_at,n_px,wbank);
                let r_q=select(vec3<f32>(0.0),clamp(h_q.s/max(h_q.g,vec3<f32>(1e-6)),vec3<f32>(0.0),vec3<f32>(4.0)),any(h_q.s>vec3<f32>(0.0)) && any(h_q.g>vec3<f32>(1e-6)));
                let y_q=clamp(fg_luminance(r_q),0.0,1.0);
                m1+=y_q;m2+=y_q*y_q;mn+=1.0;
                if h_q.pen>=0.0 { pen_eff=select(min(pen_eff,h_q.pen),h_q.pen,pen_eff<0.0); }
            }
        }
        let mean_y=m1/max(mn,1.0);
        let sp_var=max(m2/max(mn,1.0)-mean_y*mean_y,0.0);
        let bern=mean_y*(1.0-mean_y);
        // Zero variance when 3x3 is completely unanimous (m1==0 or m1==mn -> exact 0 or 1),
        // except a Laplace floor on cold cuts (0.15/n^2) and a tighter floor on moving-emitter
        // 1-frame penumbrae (0.08/max(n^2,16): dissolves 0/4 dark stipple clusters into penumbra
        // without bleeding across 0->1 shadow edges).
        let cut_floor=select(0.0,0.15/max(h_c.n*h_c.n,1.0),cold_cut && (pen_eff>10.0*world_per_px || (h_c.n<=4.5 && pen_eff>3.0*world_per_px)));
        let move_floor=select(0.0,0.08/max(h_c.n*h_c.n,16.0),fg_emitter_moved() && h_c.n<=4.5 && pen_eff>4.0*world_per_px);
        let cold_floor=max(cut_floor,move_floor);
        var_c=max(select(0.0,max(sp_var,bern/max(h_c.n,1.0)),bern>1e-4 || sp_var>1e-5),cold_floor);
        if var_c<1e-5 && h_c.n>=8.0 {
            fg_store_atrous_tap(dst_idx,vec4<f32>(r_c,0.0));
            return;
        }
    }

    let y_c=fg_luminance(r_c);
    // On passes 1..4, outer-penumbra pixels that never directly hit a blocker (`pen_c < 0`)
    // inherit the physical blocker penumbra width from their 4 same-plane cardinal neighbors at
    // `step` so wide grazing penumbrae (>60 px) stay smooth while sharp contact shadows stay crisp.
    if pass_idx>0u && pen_eff<0.0 && y_c>0.005 && y_c<0.995 && h_c.n<16.0 {
        let card=array<vec2<i32>,4>(vec2<i32>(step,0),vec2<i32>(-step,0),vec2<i32>(0,step),vec2<i32>(0,-step));
        for(var ci=0u;ci<4u;ci+=1u) {
            let q=vec2<i32>(id)+card[ci];
            if any(q<vec2<i32>(0)) || any(q>=vec2<i32>(size)) {continue;}
            let q_at=u32(q.y)*size.x+u32(q.x);
            let g_q=fg_direct_guide[fg_guide_idx(q_at,n_px,wbank)];
            let d_q=bitcast<f32>(g_q.x);
            if d_q<=0.0 {continue;}
            let n_q=fg_oct_decode(g_q.y);
            if dot(n_c,n_q)<0.94 {continue;}
            let dp=fg_world_at(vec2<u32>(q),size,d_q)-p_c;
            if max(abs(dot(n_c,dp)),abs(dot(n_q,dp)))>plane_tol {continue;}
            let qp=fg_load_direct_hist(q_at,n_px,wbank).pen;
            if qp>=0.0 { pen_eff=select(min(pen_eff,qp),qp,pen_eff<0.0); }
        }
    }
    // Keep unclamped screen-space penumbra width so crisp contact shadows (~5-20 px) and medium
    // pillar shadows (~25 px) stay sharp while ultra-wide far-blocker penumbrae (>60 px across
    // grazing walls) expand step stride by 1.5x (1,3,6,12,24) under young/whip-pan history to
    // dissolve 16-32 px clouds cleanly without washing out medium or contact shadow edges.
    let pen_px_raw=select(select(0.0,24.0,y_c>0.005 && y_c<0.995 && h_c.n<16.0),pen_eff/max(world_per_px,1e-5),pen_eff>=0.0);
    let soft_pen=smoothstep(8.0,24.0,pen_px_raw);
    let ultra_scale=select(select(0.0,0.30,fg_state[3]==0u || (fg_direct_step16() && h_c.n<=8.5 && !fg_emitter_moved())),1.0,!fg_camera_still() && fg_state[3]!=0u && !fg_emitter_moved());
    let ultra_soft=ultra_scale*smoothstep(60.0,140.0,pen_px_raw);
    let step_eff=select(step,step+(step>>1u),step>=2 && ultra_soft>0.5 && h_c.n<16.0);
    let k_base=mix(mix(mix(4.0,2.2,hist_t),mix(8.0,5.0,hist_t),soft_pen),mix(24.0,14.0,hist_t),ultra_soft);
    let k_sig=max(k_base,select(0.0,6.5,pass_idx==1u && h_c.n<3.0));
    let sigma_y=max(k_sig*sqrt(max(var_c,0.0))+mix(0.06,mix(0.02,0.035,soft_pen),hist_t)+0.05*ultra_soft*(1.0-hist_t),1e-4);
    let cold_prefilter=pass_idx==0u && (h_c.n<8.0 || (fg_direct_step16() && h_c.n<=8.5));
    let max_world_r=select(1e9,mix(1.8,1.0,hist_t)*pen_eff+2.5*world_per_px,pen_eff>=0.0);
    var sum_r=r_c;
    var sum_v=var_c;
    var sum_w=1.0;
    let kw=array<f32,3>(0.25,0.5,0.25);
    for(var dy=-1;dy<=1;dy+=1) {
        for(var dx=-1;dx<=1;dx+=1) {
            if dx==0 && dy==0 {continue;}
            let q=vec2<i32>(id)+vec2<i32>(dx*step_eff,dy*step_eff);
            if any(q<vec2<i32>(0)) || any(q>=vec2<i32>(size)) {continue;}
            // Midpoint plane continuity check at step >= 4 stops bleeding across stairs/pillars/cover.
            if step_eff>=4 {
                let mid=vec2<i32>(id)+vec2<i32>(dx*(step_eff/2),dy*(step_eff/2));
                let m_at=u32(mid.y)*size.x+u32(mid.x);
                let g_m=fg_direct_guide[fg_guide_idx(m_at,n_px,wbank)];
                let d_m=bitcast<f32>(g_m.x);
                if d_m<=0.0 {continue;}
                let n_m=fg_oct_decode(g_m.y);
                if dot(n_c,n_m)<0.94 {continue;}
                let dm=fg_world_at(vec2<u32>(mid),size,d_m)-p_c;
                if max(abs(dot(n_c,dm)),abs(dot(n_m,dm)))>plane_tol {continue;}
            }
            let q_at=u32(q.y)*size.x+u32(q.x);
            let g_q=fg_direct_guide[fg_guide_idx(q_at,n_px,wbank)];
            let d_q=bitcast<f32>(g_q.x);
            if d_q<=0.0 {continue;}
            let n_q=fg_oct_decode(g_q.y);
            let nd=dot(n_c,n_q);
            if nd<0.94 {continue;}
            let dp=fg_world_at(vec2<u32>(q),size,d_q)-p_c;
            if max(abs(dot(n_c,dp)),abs(dot(n_q,dp)))>plane_tol {continue;}
            if pen_eff>=0.0 && fg_aniso_dist(dp,n_c,p_c,1.0,0.12)>max_world_r {continue;}
            var r_q=vec3<f32>(0.0);
            var v_q=var_c;
            if pass_idx==0u {
                let h_q=fg_load_direct_hist(q_at,n_px,wbank);
                r_q=select(vec3<f32>(0.0),clamp(h_q.s/max(h_q.g,vec3<f32>(1e-6)),vec3<f32>(0.0),vec3<f32>(4.0)),any(h_q.s>vec3<f32>(0.0)) && any(h_q.g>vec3<f32>(1e-6)));
                let yq=clamp(fg_luminance(r_q),0.0,1.0);
                // Gentle Laplace floor ONLY on cold cut pixels so binary 0/1 taps
                // don't collapse variance to zero, while leaving still/whip-pan edges intact.
                let v_base=yq*(1.0-yq);
                let v_num=select(v_base,max(v_base,0.15/max(h_q.n,1.0)),cold_cut && var_c>1e-5);
                v_q=v_num/max(h_q.n,1.0);
            } else {
                let src_idx=select(fg_pong_idx(q_at,n_px),fg_ping_idx(q_at,n_px),(pass_idx&1u)==1u);
                let rv=fg_load_atrous_tap(src_idx);
                r_q=rv.xyz;
                v_q=rv.w;
            }
            let n2=nd*nd;let n4=n2*n2;let n8=n4*n4;let n16=n8*n8;
            let nw=n16*n16;
            let dy_l=abs(fg_luminance(r_q)-y_c);
            let lw=select(exp(-dy_l/sigma_y),1.0,cold_prefilter);
            let k_w=(kw[dx+1]*kw[dy+1])*4.0;
            let w=k_w*nw*lw;
            sum_r+=r_q*w;
            sum_v+=v_q*w*w;
            sum_w+=w;
        }
    }
    let inv_w=1.0/max(sum_w,1e-6);
    // Scale propagated variance for spatial correlation across à-trous passes so wide penumbrae
    // retain accurate sigma_y at step=4/8/16 instead of collapsing to zero after pass 1.
    let next_var=min(sum_v*inv_w*inv_w*2.2,max(var_c,1e-6));
    fg_store_atrous_tap(dst_idx,vec4<f32>(sum_r*inv_w,next_var));
}

@compute @workgroup_size(8,4) fn fg_direct_atrous_1(@builtin(global_invocation_id) id:vec3<u32>) { fg_direct_atrous_pass(id.xy,1,0u); }
@compute @workgroup_size(8,4) fn fg_direct_atrous_2(@builtin(global_invocation_id) id:vec3<u32>) { fg_direct_atrous_pass(id.xy,2,1u); }
@compute @workgroup_size(8,4) fn fg_direct_atrous_4(@builtin(global_invocation_id) id:vec3<u32>) { fg_direct_atrous_pass(id.xy,4,2u); }
@compute @workgroup_size(8,4) fn fg_direct_atrous_8(@builtin(global_invocation_id) id:vec3<u32>) { fg_direct_atrous_pass(id.xy,8,3u); }
@compute @workgroup_size(8,4) fn fg_direct_atrous_16(@builtin(global_invocation_id) id:vec3<u32>) { fg_direct_atrous_pass(id.xy,16,4u); }

// ============================================================================================
// Indirect screen-probe final gather (Sky L2 + Multi-Bounce L1)
// ============================================================================================
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
            let t=center+vec2<i32>(ox,oy);
            if any(t<vec2<i32>(0)) || any(t>=tiles) {continue;}
            for(var slot=0u;slot<FG_SLOTS;slot+=1u) {
                let idx=fg_tile_at(u32(t.y*tiles.x+t.x)*FG_SLOTS+slot,size,bank);
                var p=fg_plane(idx,size);
                if p.point.w<=0.0 || dot(p.normal.xyz,s.normal)<0.95 {continue;}
                let scale=max(footprint,p.normal.w);
                p.normal.w=scale;
                let geo=fg_on_plane(p,s.prev_world,s.normal);
                if geo<=0.01 {continue;}
                let d3=fg_aniso_dist(s.prev_world-p.point.xyz,s.normal,s.world,scale,0.0625);
                if d3>1.75 {continue;}
                let score=geo*exp(-d3*d3*3.0);
                if score>best {best=score;chosen=idx;}
            }
        }
    }
    return chosen;
}

fn fg_probe_luma(index:u32,size:vec2<u32>)->f32 {
    let at=fg_at(index,size);
    let sky0=bitcast<vec4<f32>>(gi_prepared[at].r0).xyz;
    let bnc0=bitcast<vec4<f32>>(gi_prepared[at+2u].r1).xyz;
    return fg_luminance(max(sky0+bnc0,vec3<f32>(0.0)));
}
// Asymmetric logarithmic energy-ratio guard with a 0.25 unexposed radiance floor:
// - Brighter neighbors (`l_p > l_c`): sharply rejected to exact 0.0 at >= 2.6x so near-emitter
//   bounce spikes, sunlit-floor bleed, and 1-ray fireflies NEVER pollute dimmer neighbors.
// - Dimmer neighbors (`l_p <= l_c`): accepted at full weight 1.0 when center history is young
//   (`c_w < 8.0`) so noisy firefly probes average cleanly into their calm neighbors instead of
//   turning into isolated 16x16 polka dots; warm settled bright centers reject only huge drops.
// - Dim grazing indirect walls (L ~ 0.01..0.2) stay at weight 1.0 (zero noise-locking mottling).
fn fg_ratio_weight(l_c:f32,l_p:f32,c_w:f32)->f32 {
    if l_p>l_c {
        let lr=log2((l_p+0.25)/(l_c+0.25));
        if lr>=1.4 {return 0.0;}
        let ex=max(lr-0.45,0.0);
        return exp(-ex*ex*8.0);
    }
    if c_w<8.0 {return 1.0;}
    let lr=log2((l_c+0.25)/(l_p+0.25));
    let ex=max(lr-1.2,0.0);
    return exp(-ex*ex*4.0);
}

fn fg_load_prev_tile(s:FgSurface,size:vec2<u32>,footprint:f32,prev_idx:u32)->FgProbe {
    var empty:FgProbe;
    if prev_idx==0xffffffffu {return empty;}
    let anchor=fg_plane(prev_idx,size);
    let anchor_shift=fg_aniso_dist(s.prev_world-anchor.point.xyz,s.normal,s.world,footprint,0.0625);
    let still_tol=select(0.05,0.08,fg_camera_still() && !s.moved);
    if anchor_shift<=still_tol {
        return fg_load(prev_idx,size);
    }
    let rp=fg_reproject_pixel(s.prev_world,size);
    if rp.z<=0.0 {return empty;}
    let tiles=vec2<i32>((size+vec2<u32>(FG_TILE-1u))/FG_TILE);
    let f=(rp.xy-vec2<f32>(0.5))/f32(FG_TILE)-vec2<f32>(0.5);
    let base=vec2<i32>(floor(f));
    let fr=f-floor(f);
    let bank=fg_read_bank();
    let anchor_luma=fg_probe_luma(prev_idx,size);
    fg_clear();
    var w_sum=0.0;
    var hist_sum=0.0;
    var pos_sum=vec3<f32>(0.0);
    var ndot_sum=0.0;
    for(var k=0u;k<4u;k+=1u) {
        let o=vec2<i32>(i32(k&1u),i32((k>>1u)&1u));
        let t=base+o;
        // Skip out-of-bounds screen borders instead of clamping (prevents edge double-weighting).
        if any(t<vec2<i32>(0)) || any(t>=tiles) {continue;}
        let bil=select(1.0-fr.x,fr.x,o.x!=0)*select(1.0-fr.y,fr.y,o.y!=0);
        if bil<=1e-4 {continue;}
        var best_geo=0.0;
        var best_idx=0xffffffffu;
        var best_p:FgPlane;
        var best_ndot=0.0;
        var best_d3=1e9;
        for(var slot=0u;slot<FG_SLOTS;slot+=1u) {
            let idx=fg_tile_at(u32(t.y*tiles.x+t.x)*FG_SLOTS+slot,size,bank);
            var p=fg_plane(idx,size);
            let ndot=dot(p.normal.xyz,s.normal);
            if p.point.w<=0.0 || ndot<0.95 {continue;}
            let scale=max(footprint,p.normal.w);
            p.normal.w=scale;
            let geo=fg_on_plane(p,s.prev_world,s.normal);
            if geo<=0.01 {continue;}
            let d3=fg_aniso_dist(s.prev_world-p.point.xyz,s.normal,s.world,scale,0.0625);
            if d3>1.75 {continue;}
            if geo>best_geo {best_geo=geo;best_idx=idx;best_p=p;best_ndot=ndot;best_d3=d3;}
        }
        if best_idx==0xffffffffu {continue;}
        let pos_w=exp(-max(best_d3-1.0,0.0)*max(best_d3-1.0,0.0)*2.0);
        let lw=fg_ratio_weight(anchor_luma,fg_probe_luma(best_idx,size),anchor.point.w);
        let w=bil*best_geo*pos_w*lw;
        if w<=1e-5 {continue;}
        fg_add(best_idx,size,w);
        w_sum+=w;
        hist_sum+=w*best_p.point.w;
        pos_sum+=w*best_p.point.xyz;
        ndot_sum+=w*best_ndot;
    }
    let max_ext=0.95;
    let move_t=smoothstep(0.20,0.60,anchor_shift);
    if w_sum<=1e-4 {
        if anchor_shift>max_ext {return empty;}
        var fb=fg_load(prev_idx,size);
        let curved=dot(fb.normal.xyz,s.normal)<0.995;
        let ext_t=smoothstep(0.20,max_ext,anchor_shift);
        fb.point.w=min(fb.point.w,mix(select(8.0,3.0,curved),1.0,max(ext_t,move_t)));
        return fb;
    }
    let inv=1.0/w_sum;
    let ext_shift=fg_aniso_dist(s.prev_world-pos_sum*inv,s.normal,s.world,footprint,0.0625);
    if ext_shift>max_ext {return empty;}
    var out:FgProbe;
    for(var v=0u;v<FG_SH_VECS;v+=1u) {out.sh[v]=fg_sum[v]*inv;}
    let blur=fr.x*(1.0-fr.x)+fr.y*(1.0-fr.y);
    let blur_t=clamp(blur*2.0,0.0,1.0);
    let ext_t=smoothstep(0.20,max_ext,ext_shift);
    let curved=ndot_sum*inv<0.995;
    // Bounded bilinear history caps prevent recursive multi-frame spatial diffusion of bright
    // sunlit/near-emitter bounce into shadowed corners during 14 m/s sprints and whip-pans.
    var move_cap=mix(mix(8.0,2.5,blur_t),1.0,max(ext_t,move_t));
    if curved {move_cap=min(move_cap,3.0);}
    out.point=vec4<f32>(s.prev_world,min(hist_sum*inv,move_cap));
    out.normal=vec4<f32>(s.normal,footprint);
    return out;
}

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
        let b=base+o;
        if any(b<vec2<i32>(0)) || any(b>=blocks) {continue;}
        let idx=fg_adaptive_in(vec2<u32>(b),size,bank);
        var p=fg_plane(idx,size);
        if p.point.w<=0.0 || dot(p.normal.xyz,s.normal)<0.95 {continue;}
        let full_scale=max(footprint,p.normal.w);
        p.normal.w=full_scale;
        let geo=fg_on_plane(p,s.prev_world,s.normal);
        if geo<=0.01 {continue;}
        let d3=fg_aniso_dist(s.prev_world-p.point.xyz,s.normal,s.world,0.5*full_scale,0.0625);
        if d3>1.75 {continue;}
        let score=geo*exp(-d3*d3*4.0);
        if score>best {best=score;chosen=idx;}
    }
    if chosen!=0xffffffffu {return chosen;}
    return fg_find_prev_tile(s,size,footprint);
}

fn fg_changed_recently(word:u32,frame:u32)->bool { return ((frame-(word>>8u))&0xffffffu)<=1u; }
var<workgroup> fg_moved:atomic<u32>;
var<workgroup> fg_asked:array<atomic<u32>,2>;
var<workgroup> fg_asked_coarse:array<atomic<u32>,2>;
var<workgroup> fg_wg_cold:array<atomic<u32>,2>;
var<workgroup> fg_wg_geom:array<atomic<u32>,2>;
var<workgroup> fg_wg_geom_ans:array<atomic<u32>,2>;
var<workgroup> fg_trace_mask:array<u32,2>;
var<workgroup> fg_wg_boost:u32;
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

const FG_NEE:u32=2u;
const FG_HEMI:u32=FG_RAYS-FG_NEE;
fn fg_nee(lane:u32,ray:u32,s:FgSurface,r:u32,tile_xy:vec2<u32>) {
    let phase=fg_phase2(tile_xy,0x369cf258u);
    let u_pick=fract((f32(ray)+phase.x)/f32(FG_NEE));
    let pick=gi_emitter_pick(u_pick);
    if pick.probability<=0.0 {return;}
    let w=gi_word(GI_H_EMITTERS)+pick.index*GI_EMITTER_WORDS;
    let a=gi_emitter_vec(w);
    let e1=gi_emitter_vec(w+3u);
    let e2=gi_emitter_vec(w+6u);
    if !fg_emitter_faces(w,a,e1,e2,s.world) {return;}
    let g=fg_hash(r^((ray+1u)*0x9e3779b9u));
    var uv=vec2<f32>(f32(g&0xffffu),f32(g>>16u))*(1.0/65536.0);
    if uv.x+uv.y>1.0 {uv=vec2<f32>(1.0)-uv;}
    let origin=fma(s.normal,vec3<f32>(s.epsilon),s.world);
    let to=a+e1*uv.x+e2*uv.y-origin;
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
    let tmax=dist-max(s.epsilon*4.0,2e-3);
    if tmax>s.epsilon && rl_blocked(origin,d,s.epsilon,tmax) {return;}
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

fn fg_lane(lane:u32,ray:u32,hemi_count:u32,slot_id:u32,s:FgSurface,pixel:vec2<u32>,r:u32,tile_xy:vec2<u32>,cadence_xy:vec2<u32>,stable:bool,use_nee:bool) {
    var radiance=vec3<f32>(0.0);
    var d=vec3<f32>(0.0,0.0,1.0);
    var ray_w=1.0;
    fg_hit=false;
    var answered=false;
    for(var i=0u;i<40u;i+=1u) {fg_lane_sh[lane][i]=0.0;}
    for(var i=0u;i<12u;i+=1u) {fg_nee_sh[lane][i]=0.0;}
    if s.placement!=0u {
        if use_nee && ray<FG_NEE {
            fg_nee(lane,ray,s,fg_hash(r^(fg_frame()*0x27d4eb2fu)),tile_xy);
        } else {
            let hemi_ray=select(ray,ray-FG_NEE,use_nee);
            let dir_w=fg_direction(s.normal,hemi_ray,hemi_count,tile_xy);
            d=dir_w.xyz;
            ray_w=dir_w.w;
            let hemi_pdf=FG_HEMISPHERE_PDF/max(ray_w,1e-6);
            radiance=fg_radiance(fma(s.normal,vec3<f32>(s.epsilon),s.world),d,s.epsilon,hemi_pdf);
            atomicAdd(&fg_rays[0],1u);
            // Request missing fine cells: genuine misses (`fg_no_value`) use their own 4/probe
            // quota (`fg_asked`) into GI_BAND_PIXEL; phase-locked coarse-fallback refinements
            // (`ask_coarse`) use a separate 1/probe quota (`fg_asked_coarse`) with bit 16 ->
            // GI_BAND_TERTIARY so optional refinements never displace genuine misses or halve gathers.
            let ask_phase=select(cadence_xy.x+cadence_xy.y+fg_frame(),cadence_xy.x+cadence_xy.y*2u+fg_frame(),stable);
            let ask_coarse=!fg_no_value && (ask_phase&select(15u,31u,FG_HALF_RATE))==0u;
            if fg_hit && rl_cache_missed.x!=0xffffffffu {
                if fg_no_value && atomicAdd(&fg_asked[slot_id],1u)<4u {
                    fg_request(rl_cache_missed);
                } else if ask_coarse && atomicAdd(&fg_asked_coarse[slot_id],1u)<1u {
                    fg_request(vec4<u32>(rl_cache_missed.x|0x10000u,rl_cache_missed.yzw));
                }
            }
            if fg_hit {
                atomicAdd(&fg_wg_geom[slot_id],1u);
                if !fg_unanswered {atomicAdd(&fg_wg_geom_ans[slot_id],1u);}
            }
            if fg_hit && (fg_unanswered || fg_changing) {atomicAdd(&fg_wg_cold[slot_id],1u);}
            answered=!(fg_hit && fg_unanswered);
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

fn fg_reduce(lane:u32,boosted:bool) {
    let span=select(FG_RAYS,2u*FG_RAYS,boosted);
    let first=select((lane/FG_RAYS)*FG_RAYS,0u,boosted);
    let local_l=select(lane%FG_RAYS,lane,boosted);
    var total_luma=0.0;
    var max_luma=0.0;
    var max_lane=0u;
    for(var l=0u;l<span;l+=1u) {
        let c=vec3<f32>(fg_lane_sh[first+l][FG_BOUNCE],fg_lane_sh[first+l][FG_BOUNCE+1u],fg_lane_sh[first+l][FG_BOUNCE+2u]);
        let y=dot(max(c,vec3<f32>(0.0)),vec3<f32>(0.2126,0.7152,0.0722));
        total_luma+=y;
        if y>max_luma {max_luma=y;max_lane=l;}
    }
    let rest_mean=max(total_luma-max_luma,0.0)/max(f32(span-1u),1.0);
    let firefly_cap=180.0*rest_mean;
    let cap_scale=select(1.0,min(1.0,firefly_cap/max(max_luma,1e-30)),max_luma>firefly_cap && rest_mean>1e-7);
    workgroupBarrier();
    for(var i=local_l;i<52u;i+=span) {
        var sum=0.0;
        if i<40u {
            for(var l=0u;l<span;l+=1u) {
                let w=select(1.0,cap_scale,l==max_lane && i>=FG_BOUNCE);
                sum+=fg_lane_sh[first+l][i]*w;
            }
            fg_lane_sh[first][i]=sum;
        } else if !boosted {
            for(var l=0u;l<FG_NEE;l+=1u) {sum+=fg_nee_sh[first+l][i-40u];}
            fg_nee_sh[first][i-40u]=sum;
        }
    }
}

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

// Double-buffered, reprojected change detector: reads `(word, usual)` from the reprojected
// previous probe in `read_bank` and writes to `state_index` in `write_bank` (zero race and zero
// cross-geometry aliasing under 10 deg/frame whip-pans). Higher `usual` ceiling (0.55) prevents
// dark grazing walls from repeatedly flushing history on normal 32-ray Monte Carlo variance.
const FG_CHANGE_LO:f32=2.2;
const FG_CHANGE_HI:f32=5.0;
const FG_RESTART:f32=1.5;
fn fg_adapt_history(state_index:u32,prev_state_index:u32,size:vec2<u32>,fresh:array<vec4<f32>,10>,old:FgProbe,count:f32,max_hist:f32,n:vec3<f32>,missing_frac:f32)->f32 {
    let w_at=fg_state_at(state_index,size,fg_write_bank());
    if w_at+1u>=arrayLength(&fg_state) {return max_hist;}
    var word=0u;
    var usual=select(0.14,0.22,FG_HALF_RATE);
    if prev_state_index!=0xffffffffu {
        let r_at=fg_state_at(prev_state_index,size,fg_read_bank());
        if r_at+1u<arrayLength(&fg_state) {
            word=fg_state[r_at];
            let u=bitcast<f32>(fg_state[r_at+1u]);
            if u>0.01 && u<=0.60 {usual=u;}
        }
    }
    if count<=1.0 || missing_frac>0.35 {
        fg_state[w_at]=word&0xffffff00u;
        fg_state[w_at+1u]=bitcast<u32>(select(0.14,0.22,FG_HALF_RATE));
        return max_hist;
    }
    let e_new=fg_irradiance(fresh,n);
    let e_old=fg_irradiance(old.sh,n);
    let peak=max(e_new,e_old);
    if peak<=1e-4 {
        let quiet=select(word&0x3fu,min((word&0x3fu)+1u,63u),old.point.w>=FG_HALF_MIN);
        fg_state[w_at]=(word&0xffffff00u)|quiet;
        fg_state[w_at+1u]=bitcast<u32>(usual);
        return max_hist;
    }
    let r=abs(e_new-e_old)/peak;
    let log_diff=abs(log2(max(e_new,1e-4))-log2(max(e_old,1e-4)));
    let z=max(r/max(usual,0.06),log_diff*2.2);
    let decay=smoothstep(FG_CHANGE_LO,FG_CHANGE_HI,z);
    let sign_bit=select(0u,128u,e_new>e_old);
    let prev_pending=(word&64u)!=0u;
    let prev_sign=word&128u;
    let confirmed=prev_pending && prev_sign==sign_bit;
    if old.point.w>=FG_RESTART && decay>0.0 {
        if confirmed || z>=3.8 {
            fg_state[w_at]=((fg_frame()&0xffffffu)<<8u)|64u|sign_bit;
            fg_state[w_at+1u]=bitcast<u32>(usual);
            return mix(max_hist,FG_RESTART,decay);
        }
        fg_state[w_at]=(word&0xffffff3fu)|64u|sign_bit;
        fg_state[w_at+1u]=bitcast<u32>(usual);
        if z>=2.6 {
            return mix(max_hist,FG_RESTART,decay*0.30);
        }
        return max_hist;
    }
    usual=clamp(mix(usual,r,0.125),0.04,0.55);
    let quiet=select(word&0x3fu,min((word&0x3fu)+1u,63u),old.point.w>=FG_HALF_MIN);
    fg_state[w_at]=(word&0xffffff00u)|quiet;
    fg_state[w_at+1u]=bitcast<u32>(usual);
    return max_hist;
}

fn fg_accumulate(write_index:u32,prev_index:u32,state_index:u32,size:vec2<u32>,probe_slot:u32,s:FgSurface,pixel:vec2<u32>,bilinear_tile:bool,boosted:bool,use_nee:bool) {
    let first=select(probe_slot*FG_RAYS,0u,boosted);
    var sh:array<f32,40>;
    var nee:array<f32,12>;
    for(var i=0u;i<40u;i+=1u) {sh[i]=fg_lane_sh[first][i];}
    for(var i=0u;i<12u;i+=1u) {nee[i]=select(0.0,fg_nee_sh[first][i],use_nee);}
    let footprint=fg_footprint(pixel,size,s.world);
    let has_old=prev_index!=0xffffffffu;
    let stride=fg_bank_stride(size);
    let prev_state_idx=select(0xffffffffu,prev_index-fg_read_bank()*stride,has_old);
    var old:FgProbe;
    if has_old {
        if bilinear_tile {old=fg_load_prev_tile(s,size,footprint,prev_index);}
        else {
            old=fg_load(prev_index,size);
            let ashift=fg_aniso_dist(s.prev_world-old.point.xyz,s.normal,s.world,footprint,0.0625);
            if ashift>0.95 {
                old.point.w=0.0;
            } else if ashift>0.08 {
                let curved=dot(old.normal.xyz,s.normal)<0.995;
                let ext_t=smoothstep(0.35,0.95,ashift);
                old.point.w=min(old.point.w,mix(select(20.0,8.0,curved),5.0,ext_t));
            }
        }
    }
    let answered=sh[FG_ANSWERED];
    let has_nee=use_nee && (nee[0]+nee[1]+nee[2])>1e-6;
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
    let hemi_count=select(select(FG_RAYS,2u*FG_RAYS,boosted),FG_HEMI,use_nee);
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
            let rem=old.sh[b/4u][b%4u]-nee[i];
            let borrow=select(rem,max(rem,0.0),i<3u);
            sh[b]=sh[b]*hemi_scale+borrow*geom_missing_frac+nee[i];
        }
    } else if geom_hits>0u && geom_ans>0u {
        let bounce_scale=hemi_scale*min(f32(geom_hits)/f32(geom_ans),1.25);
        for(var i=0u;i<12u;i+=1u) {sh[FG_BOUNCE+i]=sh[FG_BOUNCE+i]*bounce_scale+nee[i];}
    } else {
        for(var i=0u;i<12u;i+=1u) {sh[FG_BOUNCE+i]=sh[FG_BOUNCE+i]*hemi_scale+nee[i];}
    }
    let emitter_moved=fg_emitter_moved();
    // Moving emitters flush old indirect bounce immediately (1.0 frame while moving, 2.0 while
    // relighting) so bright dragged light panels leave zero multi-frame bounce trails behind them.
    let max_hist=select(select(select(FG_HISTORY,16.0,s.moved),2.0,fg_recently_changed()),1.0,emitter_moved);
    let sample_w=select(1.0,2.0,boosted);
    var count=max(sample_w*(1.0-geom_missing_frac),0.25);
    if has_old && old.point.w>0.0 {
        count=min(old.point.w+sample_w,max_hist);
    }
    var fresh:array<vec4<f32>,10>;
    for(var v=0u;v<FG_SH_VECS;v+=1u) {fresh[v]=vec4<f32>(sh[v*4u],sh[v*4u+1u],sh[v*4u+2u],sh[v*4u+3u]);}
    let hist_cap=fg_adapt_history(state_index,prev_state_idx,size,fresh,old,count,max_hist,s.normal,max(missing_frac,geom_missing_frac));
    count=min(count,hist_cap);
    let cold_t=smoothstep(0.25,0.65,cold_frac);
    let warm_eff=select(max(count-6.0,2.0),min(count,2.0),count<=8.0);
    let startup_cold=fg_recently_changed() || count<=6.0;
    let eff_count=mix(count,warm_eff,select(cold_t*0.35,cold_t,startup_cold));
    let blend=clamp(sample_w/max(eff_count,sample_w),0.0,1.0);
    var out:FgProbe;
    for(var v=0u;v<FG_SH_VECS;v+=1u) {out.sh[v]=select(fresh[v],mix(old.sh[v],fresh[v],blend),has_old && old.point.w>0.0 && count>sample_w);}
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
    if lane<2u {atomicStore(&fg_asked[lane],0u);atomicStore(&fg_asked_coarse[lane],0u);atomicStore(&fg_rays[lane],0u);atomicStore(&fg_wg_cold[lane],0u);atomicStore(&fg_wg_geom[lane],0u);atomicStore(&fg_wg_geom_ans[lane],0u);}
    if lane==0u {atomicStore(&fg_moved,0u);fg_wg_boost=0u;}
    workgroupBarrier();
    // Read neighbor change flags strictly from `read_bank` (zero intra-dispatch race!).
    let rbank=fg_read_bank();
    if lane<9u {
        let t=vec2<i32>(group.xy)+vec2<i32>(i32(lane%3u)-1,i32(lane/3u)-1);
        if all(t>=vec2<i32>(0)) && all(t<vec2<i32>(tiles)) {
            let near=u32(t.y)*tiles.x+u32(t.x);
            for(var k=0u;k<FG_SLOTS;k+=1u) {
                let at=fg_state_at(near*FG_SLOTS+k,size,rbank);
                if at<arrayLength(&fg_state) && ((fg_state[at]&64u)!=0u || fg_changed_recently(fg_state[at],frame)) {atomicOr(&fg_moved,1u);}
            }
        }
    }
    workgroupBarrier();
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
    var second=FgSurface();
    var second_pixel=vec2<u32>(0u);
    if first.placement!=0u {
        var probe:FgPlane;
        probe.point=vec4<f32>(first.world,1.0);
        probe.normal=vec4<f32>(first.normal,fg_footprint(first_pixel,size,first.world));
        for(var k=0u;k<16u;k+=1u) {
            let q=group.xy*FG_TILE+vec2<u32>((k&3u)*4u+1u,(k>>2u)*4u+1u);
            let c=fg_surface_at(q,size,false);
            if c.placement!=0u && fg_on_plane(probe,c.world,c.normal)<0.37 {second=c;second_pixel=q;break;}
        }
    }
    var s=first;
    var pixel=first_pixel;
    if slot==1u {s=second;pixel=second_pixel;}
    let tile_slot=tile*FG_SLOTS+slot;
    let write_idx=fg_tile_at(tile_slot,size,fg_write_bank());
    let footprint=select(0.0,fg_footprint(pixel,size,s.world),s.placement!=0u);
    let direct_on=fg_direct_emitters();
    let use_nee=gi_word(GI_H_EMITTER_COUNT)>0u && !direct_on;
    let cur_mode=select(select(0u,2u,use_nee),1u,direct_on);
    let mode_flip=fg_state[40u]!=cur_mode && (fg_state[40u]==2u || cur_mode==2u);
    let prev_idx=select(0xffffffffu,fg_find_prev_tile(s,size,footprint),s.placement!=0u && !mode_flip);
    var prev_plane:FgPlane;
    if prev_idx!=0xffffffffu {prev_plane=fg_plane(prev_idx,size);}
    let old_w=prev_plane.point.w;
    let shift_dist=select(1e9,fg_aniso_dist(s.prev_world-prev_plane.point.xyz,s.normal,s.world,footprint,0.0625),prev_idx!=0xffffffffu);
    let still_tol=select(0.05,0.08,fg_camera_still() && !s.moved);
    let probe_shifted=prev_idx!=0xffffffffu && shift_dist>still_tol;
    let prev_state_idx=select(0xffffffffu,prev_idx-rbank*fg_bank_stride(size),prev_idx!=0xffffffffu);
    let r_at=select(0xffffffffu,fg_state_at(prev_state_idx,size,rbank),prev_state_idx!=0xffffffffu);
    let emitter_moved=fg_emitter_moved();
    let stable=!fg_recently_changed() && !emitter_moved && !probe_shifted && old_w>=28.0 && r_at<arrayLength(&fg_state) && (fg_state[r_at]&0x3fu)>=FG_STABLE && atomicLoad(&fg_moved)==0u;
    let turn=select((group.x+group.y+frame)&1u,(group.x+2u*group.y+frame)%FG_STABLE_RATE,stable);
    // On Non-RT, never skip tracing when a probe shifted (>0.10 tile footprints) or lost history,
    // preventing whip-pan & 14 m/s sprint indirect trails while keeping still/slow half-rate wins.
    let keep=!emitter_moved && !mode_flip && !fg_recently_changed() && shift_dist<=0.10 && atomicLoad(&fg_moved)==0u && FG_HALF_RATE && turn!=0u && s.placement!=0u && old_w>=FG_HALF_MIN;
    // Free 2x ray boost on history-less whip-pan strip / cut / moving-emitter probes: when slot 1
    // is empty (~75% of screen tiles), repurpose slot 1's idle lanes to trace 64 RT / 32 Non-RT rays!
    if lane==0u && !use_nee && first.placement!=0u && second.placement==0u && !keep && (prev_idx==0xffffffffu || old_w<3.0 || emitter_moved || fg_recently_changed()) {
        fg_wg_boost=1u;
    }
    let boosted=workgroupUniformLoad(&fg_wg_boost)!=0u;
    var traced=s;
    if keep {traced.placement=0u;}
    if lane%FG_RAYS==0u {fg_trace_mask[slot]=u32(traced.placement!=0u);}
    let trace_mask=workgroupUniformLoad(&fg_trace_mask);
    if boosted {
        fg_lane(lane,lane,2u*FG_RAYS,0u,first,first_pixel,fg_hash(tile*FG_SLOTS),group.xy,group.xy,false,false);
        workgroupBarrier();
        fg_reduce(lane,true);
        workgroupBarrier();
        fg_count_rays(lane);
    } else if trace_mask[0]!=0u || trace_mask[1]!=0u {
        let hemi_count=select(FG_RAYS,FG_HEMI,use_nee);
        fg_lane(lane,lane%FG_RAYS,hemi_count,slot,traced,pixel,fg_hash(tile_slot),group.xy,group.xy,stable,use_nee);
        workgroupBarrier();
        fg_reduce(lane,false);
        workgroupBarrier();
        fg_count_rays(lane);
    }
    if lane%FG_RAYS!=0u || any(group.xy>=tiles) {return;}
    if s.placement==0u {fg_invalidate(write_idx,size);return;}
    if keep {
        var carried=fg_load(prev_idx,size);
        if probe_shifted {
            let bil=fg_load_prev_tile(s,size,footprint,prev_idx);
            if bil.point.w>0.0 { carried=bil; }
        }
        carried.point=vec4<f32>(s.world,carried.point.w);
        carried.normal=vec4<f32>(s.normal,footprint);
        fg_store(write_idx,size,carried);
        let w_at=fg_state_at(tile_slot,size,fg_write_bank());
        if w_at+1u<arrayLength(&fg_state) && r_at+1u<arrayLength(&fg_state) {
            fg_state[w_at]=fg_state[r_at];
            fg_state[w_at+1u]=fg_state[r_at+1u];
        }
        return;
    }
    fg_accumulate(write_idx,prev_idx,tile_slot,size,slot,s,pixel,true,boosted && slot==0u,use_nee);
}

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

// Pass 1 of the two-pass co-planar indirect probe filter (5x5 step-1 -> fg_filtered1).
// Pure indirect sky+bounce only (direct analytical emitters are handled at pixel resolution).
// Uses symmetric history caps (1.5x) and grazing-aware anisotropic distance (min_cos = 0.0625)
// so whip-pan disocclusion edges have no 48-64 px smear wavefront and grazing walls stay smooth.
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
        let age_t=clamp((c.point.w-1.0)/23.0,0.0,1.0);
        let sigma_sh=mix(select(1.35,1.45,FG_HALF_RATE),select(1.25,1.35,FG_HALF_RATE),age_t);
        let reach=2;
        let c_luma=fg_probe_luma(index,size);
        fg_clear();
        let sh_center_w=c.point.w;
        fg_add(index,size,sh_center_w);
        var total_sh=sh_center_w;
        var eff_age_sum=sh_center_w*c.point.w;
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
                let pair_sh=select(0.85,1.0,opp_ok || !opp_in);
                let g_sh=exp(-f32(r2)/(2.0*sigma_sh*sigma_sh))*pair_sh;
                for(var k=0u;k<FG_SLOTS;k+=1u) {
                    let at=fg_tile_at(u32(t.y*tiles.x+t.x)*FG_SLOTS+k,size,bank);
                    if at==index {continue;}
                    let p=fg_plane(at,size);
                    let geo=fg_on_plane(p,c.point.xyz,c.normal.xyz);
                    if geo<=1e-4 {continue;}
                    let lw=fg_ratio_weight(c_luma,fg_probe_luma(at,size),c.point.w);
                    if lw<=1e-4 {continue;}
                    let scale=max(c.normal.w,p.normal.w);
                    let d_sh=fg_aniso_dist(p.point.xyz-c.point.xyz,c.normal.xyz,c.point.xyz,scale,0.0625);
                    let d_sh_excess=max(d_sh-1.5,0.0);
                    let dist_sh=exp(-d_sh_excess*d_sh_excess*(1.0/16.0));
                    let sh_hist_w=min(p.point.w,max(sh_center_w*3.0,6.0));
                    let w_sh=geo*dist_sh*lw*sh_hist_w*g_sh;
                    if w_sh>1e-4 {
                        fg_add(at,size,w_sh);
                        total_sh+=w_sh;
                        eff_age_sum+=w_sh*min(p.point.w,sh_hist_w);
                    }
                }
            }
        }
        var out:FgProbe;
        let inv_sh=1.0/max(total_sh,1e-6);
        out.point=vec4<f32>(c.point.xyz,clamp(eff_age_sum*inv_sh,c.point.w,FG_HISTORY));
        out.normal=c.normal;
        for(var v=0u;v<FG_SH_VECS;v+=1u) {out.sh[v]=fg_sum[v]*inv_sh;}
        fg_store(fg_filtered1(tile_slot,size),size,out);
    }
}

// Pass 2 of the co-planar indirect probe filter (3x3 at step 2 tiles -> fg_filtered):
// cascades with Pass 1 for a 9x9 co-planar indirect denoiser. Uses `fg_ratio_weight` (0.25
// unexposed radiance floor) so dim grazing walls smooth cleanly with weight 1.0 while extreme
// near-emitter bounce spikes and sunlit-vs-shadowed cliffs never bleed 4 tiles sideways.
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
        let sigma_sh2=select(1.25,1.40,FG_HALF_RATE);
        let c_luma=fg_probe_luma(index,size);
        fg_clear();
        let sh_center_w=c.point.w;
        fg_add(index,size,sh_center_w);
        var total_sh=sh_center_w;
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
                let pair_sh=select(0.85,1.0,opp_ok || !opp_in);
                let r2=dx*dx+dy*dy;
                let g_sh=exp(-f32(r2)/(2.0*sigma_sh2*sigma_sh2))*pair_sh;
                for(var k=0u;k<FG_SLOTS;k+=1u) {
                    let at=fg_filtered1(u32(t.y*tiles.x+t.x)*FG_SLOTS+k,size);
                    let p=fg_plane(at,size);
                    let geo=fg_on_plane(p,c.point.xyz,c.normal.xyz);
                    if geo<=1e-4 {continue;}
                    if dot(c.normal.xyz,p.normal.xyz)<0.995 {continue;}
                    let lw=fg_ratio_weight(c_luma,fg_probe_luma(at,size),c.point.w);
                    if lw<=1e-4 {continue;}
                    let scale=max(c.normal.w,p.normal.w);
                    let d_sh=fg_aniso_dist(p.point.xyz-c.point.xyz,c.normal.xyz,c.point.xyz,scale,0.0625);
                    let d_sh_excess=max(d_sh-2.0,0.0);
                    let dist_sh=exp(-d_sh_excess*d_sh_excess*(1.0/25.0));
                    let sh_hist_w=min(p.point.w,max(sh_center_w*3.0,6.0));
                    let w_sh=geo*dist_sh*lw*sh_hist_w*g_sh;
                    if w_sh>1e-4 {
                        fg_add(at,size,w_sh);
                        total_sh+=w_sh;
                    }
                }
            }
        }
        var out:FgProbe;
        out.point=c.point;out.normal=c.normal;
        let inv_sh=1.0/max(total_sh,1e-6);
        for(var v=0u;v<FG_SH_VECS;v+=1u) {out.sh[v]=fg_sum[v]*inv_sh;}
        fg_store(fg_filtered(tile_slot,size),size,out);
    }
}

var<workgroup> fg_orphan:atomic<u32>;
const FG_NONE:u32=0xffffffffu;
fn fg_pick_at(index:u32,size:vec2<u32>)->u32 { return gi_prepared[fg_at(index,size)+2u].r0.w; }

// Short-range sub-probe horizon ambient occlusion (reads linear depth from fg_direct_guide).
// Applied strictly to indirect sky + bounce in fg_resolve, NEVER to ray-traced direct emitters.
fn fg_corner_ao(id:vec2<u32>,size:vec2<u32>,s:FgSurface)->f32 {
    let n_px=size.x*size.y;
    if arrayLength(&fg_direct_guide)<2u*n_px {return 0.0;}
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
    let wbank=fg_write_bank();
    var occ_sum=0.0;
    let origin=vec2<f32>(id);
    for(var k=0u;k<8u;k+=1u) {
        let dir=dirs[k];
        var h_max=0.0;
        for(var step=0u;step<4u;step+=1u) {
            let q=vec2<i32>(round(origin+dir*(screen_r*fracs[step])));
            if any(q<lo) || any(q>hi) {continue;}
            let pu=vec2<u32>(q);
            let dist=bitcast<f32>(fg_direct_guide[fg_guide_idx(pu.y*size.x+pu.x,n_px,wbank)].x);
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

@compute @workgroup_size(8,8) fn fg_classify(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_id) local:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let size=textureDimensions(primary_hit);
    if lane==0u {atomicStore(&fg_orphan,FG_NONE);}
    workgroupBarrier();
    let q=group.xy*FG_BLOCK+local.xy;
    let mine=fg_surface_at(q,size,false);
    // When direct-emitter passes are skipped (zero emitters), populate linear depth in
    // fg_direct_guide right here (zero extra passes) so fg_corner_ao stays accurate on Day scenes.
    let n_px=size.x*size.y;
    if !fg_direct_emitters() && all(q<size) && arrayLength(&fg_direct_guide)>=2u*n_px {
        let dist=select(0.0,length(mine.world-primary_ray(q,size).near),mine.placement!=0u);
        fg_direct_guide[fg_guide_idx(q.y*size.x+q.x,n_px,fg_write_bank())]=vec2<u32>(bitcast<u32>(dist),0u);
    }
    // Keep even 1-pixel orphan blocks on Non-RT so curved pillars and moving thin silhouettes
    // never drop adaptive coverage during fast motion.
    if mine.placement!=0u && fg_tiles(q,size,mine,false)<=0.16 {
        let dx=i32(local.x)-4;let dy=i32(local.y)-4;
        atomicMin(&fg_orphan,(u32(dx*dx+dy*dy)<<6u)|lane);
    }
    workgroupBarrier();
    if lane!=0u {return;}
    let pick=atomicLoad(&fg_orphan);
    let index=fg_adaptive_in(group.xy,size,fg_write_bank());
    let at=fg_at(index,size)+2u;
    let c=gi_prepared[at];
    let stored=select(pick&63u,FG_NONE,pick==FG_NONE);
    gi_prepared[at]=GiPrepared(vec4<u32>(c.r0.xyz,stored),c.r1,c.r2);
    if pick==FG_NONE {fg_invalidate(index,size);}
}

var<workgroup> fg_pair:array<u32,2>;
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
    if lane%FG_RAYS==0u {fg_pair[b]=select(FG_NONE,fg_pick_at(write_idx,size),inside);atomicStore(&fg_asked[b],0u);atomicStore(&fg_asked_coarse[b],0u);atomicStore(&fg_rays[b],0u);atomicStore(&fg_wg_cold[b],0u);atomicStore(&fg_wg_geom[b],0u);atomicStore(&fg_wg_geom_ans[b],0u);}
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
    let direct_on=fg_direct_emitters();
    let use_nee=gi_word(GI_H_EMITTER_COUNT)>0u && !direct_on;
    let cur_mode=select(select(0u,2u,use_nee),1u,direct_on);
    let mode_flip=fg_state[40u]!=cur_mode && (fg_state[40u]==2u || cur_mode==2u);
    let prev_idx=select(0xffffffffu,fg_find_prev_adapt(s,size,footprint),s.placement!=0u && !mode_flip);
    var prev_plane:FgPlane;
    if prev_idx!=0xffffffffu {prev_plane=fg_plane(prev_idx,size);}
    let old_w=prev_plane.point.w;
    let rbank=fg_read_bank();
    let shift_dist=select(1e9,fg_aniso_dist(s.prev_world-prev_plane.point.xyz,s.normal,s.world,footprint,0.0625),prev_idx!=0xffffffffu);
    let still_tol=select(0.05,0.08,fg_camera_still() && !s.moved);
    let probe_shifted=prev_idx!=0xffffffffu && shift_dist>still_tol;
    let prev_state_idx=select(0xffffffffu,prev_idx-rbank*fg_bank_stride(size),prev_idx!=0xffffffffu);
    let r_at=select(0xffffffffu,fg_state_at(prev_state_idx,size,rbank),prev_state_idx!=0xffffffffu);
    let emitter_moved=fg_emitter_moved();
    let adapt_changed=r_at<arrayLength(&fg_state) && ((fg_state[r_at]&64u)!=0u || fg_changed_recently(fg_state[r_at],frame));
    let stable=!fg_recently_changed() && !emitter_moved && !probe_shifted && !adapt_changed && old_w>=28.0 && r_at<arrayLength(&fg_state) && (fg_state[r_at]&0x3fu)>=FG_STABLE;
    let turn=select((group.x+block.y+frame)&1u,(group.x+2u*block.y+frame)%FG_STABLE_RATE,stable);
    let keep=!emitter_moved && !mode_flip && !fg_recently_changed() && shift_dist<=0.25 && !adapt_changed && FG_HALF_RATE && turn!=0u && s.placement!=0u && old_w>=FG_HALF_MIN;
    var traced=s;
    if keep {traced.placement=0u;}
    if lane%FG_RAYS==0u {fg_trace_mask[b]=u32(traced.placement!=0u);}
    let trace_mask=workgroupUniformLoad(&fg_trace_mask);
    if trace_mask[0]!=0u || trace_mask[1]!=0u {
        let hemi_count=select(FG_RAYS,FG_HEMI,use_nee);
        fg_lane(lane,lane%FG_RAYS,hemi_count,b,traced,pixel,fg_hash(state_idx),clamped,vec2<u32>(group.x,block.y),stable,use_nee);
        workgroupBarrier();
        fg_reduce(lane,false);
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
        let w_at=fg_state_at(state_idx,size,fg_write_bank());
        if w_at+1u<arrayLength(&fg_state) && r_at+1u<arrayLength(&fg_state) {
            fg_state[w_at]=fg_state[r_at];
            fg_state[w_at+1u]=fg_state[r_at+1u];
        }
        return;
    }
    fg_accumulate(write_idx,prev_idx,state_idx,size,b,s,pixel,false,false,use_nee);
}

@compute @workgroup_size(8,4) fn fg_resolve(@builtin(global_invocation_id) grid:vec3<u32>) {
    if all(grid.xy==vec2<u32>(0u)) {
        fg_state[40u]=select(select(0u,2u,gi_word(GI_H_EMITTER_COUNT)>0u),1u,fg_direct_emitters());
    }
    let id=grid.xy+primary_region.xy;
    let size=textureDimensions(primary_hit);
    let s=fg_surface(id,size);
    if s.placement==0u {return;}
    let bank=fg_write_bank();
    fg_clear();
    var total=fg_tiles(id,size,s,true);
    // Smooth adaptive cross-fade over [0.04, 0.20] instead of a hard 0.08 discontinuity:
    // keeps curved cylinders and thin edges seam-free under motion.
    let adapt_ramp=1.0-smoothstep(0.04,0.20,total);
    if adapt_ramp>1e-3 {
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
                let w=adapt_ramp*exp(-dot(d,d)*0.5)*fg_on_plane(p,s.world,s.normal)*min(p.point.w,1.0);
                if w<=1e-6 {continue;}
                fg_add(index,size,w);
                total+=w;
            }
        }
    }
    if total<=1e-4 {
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
    // Apply sub-probe horizon AO ONLY to indirect sky + bounce (NEVER to ray-traced direct emitters).
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
            let delta_l1=l1_v-s.normal*(0.8660254*l0_y);
            let aniso=smoothstep(0.05,0.25,length(delta_l1)/l0_y);
            let bounce_ao=mix(1.0,clamp(1.0-0.62*occ,0.35,1.0),aniso);
            for(var k=0u;k<4u;k+=1u) { gi_rx_bounce[k]*=bounce_ao; }
        }
    }
    // Add per-pixel denoised direct analytical area-emitter light (exact Lambertian L0/L1,
    // fixing A5's 25% normal / 3.4x grazing L1 bias while keeping gi_furnace 100% exact).
    let at=id.y*size.x+id.x;
    if fg_direct_emitters() {
        let n_px=size.x*size.y;
        let u_idx=fg_u_idx(at,n_px);
        let u0=fg_direct_guide[u_idx];
        let u1=fg_direct_guide[u_idx+1u];
        let U=vec3<f32>(bitcast<f32>(u0.x),bitcast<f32>(u0.y),bitcast<f32>(u1.x));
        if any(U>vec3<f32>(0.0)) {
            let src_idx=select(fg_pong_idx(at,n_px),fg_ping_idx(at,n_px),fg_direct_step16());
            let R=select(vec3<f32>(1.0),fg_load_atrous_tap(src_idx).xyz,(u1.y>>24u)!=0u);
            let E_dir=max(U*R,vec3<f32>(0.0));
            let f_unit=fg_oct_decode(u1.y);
            // Align L1 direction to smooth `s.shading` so dot(f_eff, s.shading) == 1.0 with bounded
            // length <= 2.0: yields exact E_dir at s.shading on both flat walls and curved pillars.
            let dir_l1=mix(s.shading,f_unit,clamp(dot(f_unit,s.shading)*2.0,0.0,1.0));
            let f_eff=dir_l1/max(dot(dir_l1,s.shading),0.5);
            gi_rx_bounce[0u]+=E_dir*0.56418958;
            gi_rx_bounce[1u]+=E_dir*(0.48860251*f_eff.y);
            gi_rx_bounce[2u]+=E_dir*(0.48860251*f_eff.z);
            gi_rx_bounce[3u]+=E_dir*(0.48860251*f_eff.x);
        }
    }
    gi_rx_valid=true;
    gi_rx_absolute=true;
    gi_rx_normal=s.shading;
    let dh=fg_hash(at^(fg_frame()*0x27d4eb2fu));
    let d=((f32(dh&255u)+f32((dh>>8u)&255u))*(1.0/510.0)-0.5)*(1.0/127.0);
    gi_rx_store_with_dither(at,d);
}
