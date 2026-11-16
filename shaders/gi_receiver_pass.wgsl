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
// A covered pixel's receiver as the lookup reads it (`placement` zero: no lookup here).
struct GiRxPixel { placement:u32, input:SurfaceInput };
fn gi_rx_pixel(id:vec2<u32>,size:vec2<u32>) -> GiRxPixel {
    var p:GiRxPixel;
    if any(id>=size) || any(id>=primary_region.zw) {return p;}
    if textureLoad(primary_tiles,id/PRIMARY_TILE,0).x==0u {return p;}
    let record=textureLoad(primary_hit,vec2<i32>(id),0);
    if record.x==0u {return p;}
    let hit=primary_decode(record);
    let a=primary_input(hit.batch,hit.ia,hit.placement);
    let b=primary_input(hit.batch,hit.ib,hit.placement);
    let c=primary_input(hit.batch,hit.ic,hit.placement);
    let w=vec3<f32>(1.0-hit.bary.x-hit.bary.y,hit.bary);
    p.placement=record.x;
    p.input.receiver.world=world_position(receiver_point(a.position,b.position,c.position,w),hit.placement).xyz;
    p.input.receiver.geometric=primary_geometric(a.position,b.position,c.position,hit.placement);
    p.input.normal=vertex_output(a).normal*w.x+vertex_output(b).normal*w.y+vertex_output(c).normal*w.z;
    return p;
}
// The prepared lookup (gi_rx_*) as the 48-byte record.
struct GiRxRecord { r0:vec4<u32>, r1:vec4<u32>, r2:vec4<u32> };
fn gi_rx_record() -> GiRxRecord {
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
    return GiRxRecord(vec4<u32>(bitcast<vec2<u32>>(vec2<f32>(select(-1.0,sky,gi_rx_valid),bounce)),q0,q1),
        vec4<u32>(q2,q3,q4,q5),vec4<u32>(q6,q7,q8,q9));
}
fn gi_rx_store(at:u32,r:GiRxRecord) {
    gi_prepared[at].r0=r.r0;gi_prepared[at].r1=r.r1;gi_prepared[at].r2=r.r2;
}
@compute @workgroup_size(8,4) fn gi_receiver_pass(@builtin(global_invocation_id) grid:vec3<u32>) {
    let id=grid.xy+primary_region.xy;
    let size=textureDimensions(primary_hit);
    let p=gi_rx_pixel(id,size);
    if p.placement==0u {return;}
    gi_prepare(p.input);
    gi_rx_store(id.y*size.x+id.x,gi_rx_record());
}

// Quad sharing (`RENDERLAB_GI_RX_QUAD=0` dispatches gi_receiver_pass): where the four pixels
// of a 2x2 quad lie on one placement, in one normal bin, at the same levels and in the same
// cell of the first level, its top-left pixel looks up and the others take its record. They
// would read the same cells; only their weights inside the cell differ, by a pixel's step of
// the cache's interpolation. Elsewhere every pixel looks up its own.
// The lookup's cell at a receiver: placement, bin and levels, then the first level's base
// corner (gi_sample's), or all ones without a receiver.
fn gi_rx_signature(p:GiRxPixel) -> array<vec4<u32>,2> {
    if p.placement==0u {return array<vec4<u32>,2>(vec4<u32>(0xffffffffu),vec4<u32>(0xffffffffu));}
    let world=p.input.receiver.world;
    let bin=gi_bin(gi_bin_source(gi_viewed_normal(world,p.input.receiver.geometric),p.input.normal));
    let pick=gi_levels(world,0.0);
    let base=bitcast<vec3<u32>>(gi_corners(gi_lattice(world,gi_edge(pick.lo),bin),bin).base);
    return array<vec4<u32>,2>(vec4<u32>(p.placement,bin|(pick.lo<<8u)|(pick.hi<<16u),base.xy),vec4<u32>(base.z,0u,0u,0u));
}
var<workgroup> gi_quad_signatures:array<array<vec4<u32>,2>,32>;
var<workgroup> gi_quad_records:array<GiRxRecord,8>;
@compute @workgroup_size(8,4) fn gi_receiver_quad(@builtin(global_invocation_id) grid:vec3<u32>,
        @builtin(local_invocation_id) local:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let id=grid.xy+primary_region.xy;
    let size=textureDimensions(primary_hit);
    let p=gi_rx_pixel(id,size);
    let signature=gi_rx_signature(p);
    gi_quad_signatures[lane]=signature;
    workgroupBarrier();
    let corner=(local.y&~1u)*8u+(local.x&~1u);
    let quad=(local.y/2u)*4u+local.x/2u;
    var shared_quad=p.placement!=0u;
    for(var k=0u;k<4u;k+=1u) {
        let other=gi_quad_signatures[corner+(k&1u)+(k>>1u)*8u];
        shared_quad=shared_quad && all(other[0]==signature[0]) && all(other[1]==signature[1]);
    }
    let leads=lane==corner;
    if p.placement!=0u && (leads || !shared_quad) {
        gi_prepare(p.input);
        let r=gi_rx_record();
        gi_rx_store(id.y*size.x+id.x,r);
        if shared_quad {gi_quad_records[quad]=r;}
    }
    workgroupBarrier();
    if shared_quad && !leads {gi_rx_store(id.y*size.x+id.x,gi_quad_records[quad]);}
}
