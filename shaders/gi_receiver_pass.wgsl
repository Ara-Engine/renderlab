// GI receiver output: 48 bytes/pixel, two f32 scales and 39 snorm8 coefficients.
// Three vector words retain the original byte layout, with vector loads/stores.
struct GiPrepared { r0:vec4<u32>, r1:vec4<u32>, r2:vec4<u32> };
@group(0) @binding(68) var<storage,read_write> gi_prepared:array<GiPrepared>;
fn gi_pass_load(input:SurfaceInput,id:vec2<u32>,size:vec2<u32>) {
    let at=id.y*size.x+id.x;
    let a=gi_prepared[at].r0;let b=gi_prepared[at].r1;let c=gi_prepared[at].r2;
    let scales=bitcast<vec2<f32>>(a.xy);
    gi_rx_valid=scales.x>=0.0;
    gi_rx_normal=unit_normal(gi_bin_source(gi_viewed_normal(input.receiver.world,input.receiver.geometric),input.normal));
    let p0=unpack4x8snorm(a.z);
    let p1=unpack4x8snorm(a.w);
    let p2=unpack4x8snorm(b.x);
    let p3=unpack4x8snorm(b.y);
    let p4=unpack4x8snorm(b.z);
    let p5=unpack4x8snorm(b.w);
    let p6=unpack4x8snorm(c.x);
    let p7=unpack4x8snorm(c.y);
    let p8=unpack4x8snorm(c.z);
    let p9=unpack4x8snorm(c.w);
    gi_rx_sky[0u]=vec3<f32>(p0.x,p0.y,p0.z)*scales.x;
    gi_rx_sky[1u]=vec3<f32>(p0.w,p1.x,p1.y)*scales.x;
    gi_rx_sky[2u]=vec3<f32>(p1.z,p1.w,p2.x)*scales.x;
    gi_rx_sky[3u]=vec3<f32>(p2.y,p2.z,p2.w)*scales.x;
    gi_rx_sky[4u]=vec3<f32>(p3.x,p3.y,p3.z)*scales.x;
    gi_rx_sky[5u]=vec3<f32>(p3.w,p4.x,p4.y)*scales.x;
    gi_rx_sky[6u]=vec3<f32>(p4.z,p4.w,p5.x)*scales.x;
    gi_rx_sky[7u]=vec3<f32>(p5.y,p5.z,p5.w)*scales.x;
    gi_rx_sky[8u]=vec3<f32>(p6.x,p6.y,p6.z)*scales.x;
    gi_rx_bounce[0u]=vec3<f32>(p6.w,p7.x,p7.y)*scales.y;
    gi_rx_bounce[1u]=vec3<f32>(p7.z,p7.w,p8.x)*scales.y;
    gi_rx_bounce[2u]=vec3<f32>(p8.y,p8.z,p8.w)*scales.y;
    gi_rx_bounce[3u]=vec3<f32>(p9.x,p9.y,p9.z)*scales.y;
}
// ---- GI receiver compute entry ----
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
    let at=id.y*size.x+id.x;
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
}
