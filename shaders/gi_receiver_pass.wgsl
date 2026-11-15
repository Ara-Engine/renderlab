// GI receiver output: 64 bytes/pixel, two f32 scales and 39 snorm8 coefficients in three vector
// words (vector loads/stores), then the receiver's world position and (placement << 3 | bin)
// for the half-rate blend (gi_receiver_blend; 0xffffffff: no receiver). Shading reads r0..r2.
struct GiPrepared { r0:vec4<u32>, r1:vec4<u32>, r2:vec4<u32>, r3:vec4<u32> };
@group(0) @binding(68) var<storage,read_write> gi_prepared:array<GiPrepared>;
fn gi_pass_load(input:SurfaceInput,id:vec2<u32>,size:vec2<u32>) {
    let at=id.y*size.x+id.x;
    let a=gi_prepared[at].r0;let b=gi_prepared[at].r1;let c=gi_prepared[at].r2;
    let scales=bitcast<vec2<f32>>(a.xy);
    gi_rx_valid=scales.x>=0.0;
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
    gi_rx_bounce[0u]=vec3<f32>(unpack4x8snorm(c.x).w,unpack4x8snorm(c.y).x,unpack4x8snorm(c.y).y)*scales.y;
    gi_rx_bounce[1u]=vec3<f32>(unpack4x8snorm(c.y).z,unpack4x8snorm(c.y).w,unpack4x8snorm(c.z).x)*scales.y;
    gi_rx_bounce[2u]=vec3<f32>(unpack4x8snorm(c.z).y,unpack4x8snorm(c.z).z,unpack4x8snorm(c.z).w)*scales.y;
    gi_rx_bounce[3u]=vec3<f32>(unpack4x8snorm(c.w).x,unpack4x8snorm(c.w).y,unpack4x8snorm(c.w).z)*scales.y;
}
// ---- GI receiver compute entry ----
// The receiver at `id` (primary_decode and the vertex inputs, as shading reconstructs it) into
// rx_input and rx_placement; false where the pixel has no hit or lies outside the region.
var<private> rx_input:SurfaceInput;
var<private> rx_placement:u32;
fn gi_rx_receiver(id:vec2<u32>,size:vec2<u32>)->bool {
    if any(id>=size) || any(id>=primary_region.zw) || any(id<primary_region.xy) {return false;}
    if textureLoad(primary_tiles,id/PRIMARY_TILE,0).x==0u {return false;}
    let record=textureLoad(primary_hit,vec2<i32>(id),0);
    if record.x==0u {return false;}
    let hit=primary_decode(record);
    let a=primary_input(hit.batch,hit.ia,hit.placement);
    let b=primary_input(hit.batch,hit.ib,hit.placement);
    let c=primary_input(hit.batch,hit.ic,hit.placement);
    let w=vec3<f32>(1.0-hit.bary.x-hit.bary.y,hit.bary);
    rx_input.receiver.world=world_position(receiver_point(a.position,b.position,c.position,w),hit.placement).xyz;
    rx_input.receiver.geometric=primary_geometric(a.position,b.position,c.position,hit.placement);
    rx_input.normal=vertex_output(a).normal*w.x+vertex_output(b).normal*w.y+vertex_output(c).normal*w.z;
    rx_placement=hit.placement;
    return true;
}
// The receiver's lookup bin, as gi_prepare picks it.
fn gi_rx_bin()->u32 {
    return gi_bin(gi_bin_source(gi_viewed_normal(rx_input.receiver.world,rx_input.receiver.geometric),rx_input.normal));
}
const GI_RX_NONE:u32=0xffffffffu;
// Stores the prepared receiver (gi_rx_*) at `at`, with its half-rate key.
fn gi_rx_write(at:u32,bin:u32) {
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
    let q9=pack4x8snorm(vec4<f32>(gi_rx_bounce[3u].x,gi_rx_bounce[3u].y,gi_rx_bounce[3u].z,0.0)/vec4<f32>(bs,bs,bs,bs));
    gi_prepared[at].r0=vec4<u32>(bitcast<vec2<u32>>(vec2<f32>(select(-1.0,sky,gi_rx_valid),bounce)),q0,q1);
    gi_prepared[at].r1=vec4<u32>(q2,q3,q4,q5);
    gi_prepared[at].r2=vec4<u32>(q6,q7,q8,q9);
    gi_prepared[at].r3=vec4<u32>(bitcast<vec3<u32>>(rx_input.receiver.world),(rx_placement<<3u)|bin);
}
@compute @workgroup_size(8,4) fn gi_receiver_pass(@builtin(global_invocation_id) grid:vec3<u32>) {
    let id=grid.xy+primary_region.xy;
    let size=textureDimensions(primary_hit);
    if !gi_rx_receiver(id,size) {return;}
    gi_prepare(rx_input);
    gi_rx_write(id.y*size.x+id.x,gi_rx_bin());
}

// ---- Half-rate lookup (RENDERLAB_GI_RX_HALF, src/gi_receiver_pass.rs) ----
// The lookup is a chain of dependent hash and record loads, and a cell spans several pixels, so
// neighbouring pixels mostly read the same cells. gi_receiver_reps looks the cache up for the
// even pixel of every 2x2 block (relative to the region) in a quarter-size dispatch;
// gi_receiver_blend then gives every other pixel the bilinear blend of its nearest such
// representatives that lie on its surface: the same placement and lookup bin, and within
// about 14 degrees of its tangent plane. A pixel without one looks the cache up itself, so
// silhouettes, creases and bin changes keep the full-rate value.
@compute @workgroup_size(8,4) fn gi_receiver_reps(@builtin(global_invocation_id) grid:vec3<u32>) {
    let id=grid.xy*2u+primary_region.xy;
    let size=textureDimensions(primary_hit);
    if any(id>=size) {return;}
    let at=id.y*size.x+id.x;
    if !gi_rx_receiver(id,size) {gi_prepared[at].r3=vec4<u32>(GI_RX_NONE);return;}
    gi_prepare(rx_input);
    gi_rx_write(at,gi_rx_bin());
}
// Adds w times the stored record at `at` to gi_rx_sky and gi_rx_bounce.
fn gi_rx_accumulate(at:u32,w:f32) {
    let a=gi_prepared[at].r0;let b=gi_prepared[at].r1;let c=gi_prepared[at].r2;
    let scales=bitcast<vec2<f32>>(a.xy)*w;
    let sa=unpack4x8snorm(a.z);let sb=unpack4x8snorm(a.w);let sc=unpack4x8snorm(b.x);let sd=unpack4x8snorm(b.y);
    let se=unpack4x8snorm(b.z);let sf=unpack4x8snorm(b.w);let sg=unpack4x8snorm(c.x);let sh=unpack4x8snorm(c.y);
    let si=unpack4x8snorm(c.z);let sj=unpack4x8snorm(c.w);
    gi_rx_sky[0u]+=sa.xyz*scales.x;
    gi_rx_sky[1u]+=vec3<f32>(sa.w,sb.x,sb.y)*scales.x;
    gi_rx_sky[2u]+=vec3<f32>(sb.z,sb.w,sc.x)*scales.x;
    gi_rx_sky[3u]+=sc.yzw*scales.x;
    gi_rx_sky[4u]+=sd.xyz*scales.x;
    gi_rx_sky[5u]+=vec3<f32>(sd.w,se.x,se.y)*scales.x;
    gi_rx_sky[6u]+=vec3<f32>(se.z,se.w,sf.x)*scales.x;
    gi_rx_sky[7u]+=sf.yzw*scales.x;
    gi_rx_sky[8u]+=sg.xyz*scales.x;
    gi_rx_bounce[0u]+=vec3<f32>(sg.w,sh.x,sh.y)*scales.y;
    gi_rx_bounce[1u]+=vec3<f32>(sh.z,sh.w,si.x)*scales.y;
    gi_rx_bounce[2u]+=si.yzw*scales.y;
    gi_rx_bounce[3u]+=sj.xyz*scales.y;
}
@compute @workgroup_size(8,4) fn gi_receiver_blend(@builtin(global_invocation_id) grid:vec3<u32>) {
    let id=grid.xy+primary_region.xy;
    let size=textureDimensions(primary_hit);
    let rel=id-primary_region.xy;
    // The representatives themselves were written by gi_receiver_reps.
    if all((rel&vec2<u32>(1u))==vec2<u32>(0u)) {return;}
    if !gi_rx_receiver(id,size) {return;}
    let bin=gi_rx_bin();
    let key=(rx_placement<<3u)|bin;
    let world=rx_input.receiver.world;
    let n=normalize(rx_input.receiver.geometric);
    for(var i=0u;i<9u;i+=1u) {gi_rx_sky[i]=vec3<f32>(0.0);}
    for(var i=0u;i<4u;i+=1u) {gi_rx_bounce[i]=vec3<f32>(0.0);}
    let base=rel>>vec2<u32>(1u);
    let f=vec2<f32>(rel&vec2<u32>(1u))*0.5;
    var total=0.0;
    for(var k=0u;k<4u;k+=1u) {
        let o=vec2<u32>(k&1u,k>>1u);
        let wx=select(1.0-f.x,f.x,o.x==1u);let wy=select(1.0-f.y,f.y,o.y==1u);
        let w=wx*wy;
        if w<=0.0 {continue;}
        let rid=(base+o)*2u+primary_region.xy;
        if any(rid>=size) || any(rid>=primary_region.zw) {continue;}
        let at=rid.y*size.x+rid.x;
        let rep=gi_prepared[at].r3;
        if rep.w!=key {continue;}
        // A representative without a cache value, or off this pixel's plane, is no stand-in.
        if bitcast<f32>(gi_prepared[at].r0.x)<0.0 {continue;}
        let d=bitcast<vec3<f32>>(rep.xyz)-world;
        if abs(dot(n,d))>0.25*length(d)+1e-6 {continue;}
        gi_rx_accumulate(at,w);
        total+=w;
    }
    if total>0.0 {
        let inv=1.0/total;
        for(var i=0u;i<9u;i+=1u) {gi_rx_sky[i]*=inv;}
        for(var i=0u;i<4u;i+=1u) {gi_rx_bounce[i]*=inv;}
        gi_rx_valid=true;
    } else {
        gi_prepare(rx_input);
    }
    gi_rx_write(id.y*size.x+id.x,bin);
}
