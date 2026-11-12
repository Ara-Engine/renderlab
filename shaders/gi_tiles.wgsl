// ---- GI tile corners (RENDERLAB_GI_TILE=pass; the Mac thread's) ----
// The camera's cache lookup once per 4x4 tile corner instead of once per pixel. A compact
// pre-pass (gi_tile_corners, one thread per corner pixel (4i, 4j), clamped to the image)
// decodes each corner's receiver and runs the usual lookup (gi_prepare), so only a
// sixteenth of the shading's SIMD groups pay a lookup's latency. Shading then reads its
// tile's four corner records (gi_corner_take): a pixel that agrees with all four (each a
// hit with a value, one cache bin, normals within GI_TILE_COS, its position within
// GI_TILE_PLANE of the eye distance from the corners' bilinear surface) blends their sums
// bilinearly; any other pixel (edges, corners, contacts, bin changes) looks itself up as
// before. The SH is still evaluated at each pixel's own normal. Composed into traced
// stages only (src/program.rs), with the records at group 0 binding 68.
//
// Record, 32 words: 0-19 the 27 sky and 12 bounce SH halves (gi_rx_sky, gi_rx_bounce),
// 20-22 the receiver point, 23 the bin-source normal (octahedral snorm16 x2), 24 its bin,
// 25 flags (1 a hit, 2 the lookup found a value).
@group(0) @binding(68) var<storage,read_write> gi_corner_records:array<u32>;
const GI_CORNER_WORDS:u32=32u;

// The corners across the image: one per 4 pixels, plus the far edge.
fn gi_corner_grid(size:vec2<u32>) -> vec2<u32> { return (size+vec2<u32>(3u))/4u+vec2<u32>(1u); }
fn gi_oct(n:vec3<f32>) -> u32 {
    let p=n.xy/(abs(n.x)+abs(n.y)+abs(n.z));
    let q=select(p,(vec2<f32>(1.0)-abs(p.yx))*select(vec2<f32>(-1.0),vec2<f32>(1.0),p>=vec2<f32>(0.0)),n.z<0.0);
    return pack2x16snorm(q);
}
fn gi_unoct(word:u32) -> vec3<f32> {
    let e=unpack2x16snorm(word);
    var v=vec3<f32>(e.x,e.y,1.0-abs(e.x)-abs(e.y));
    if v.z<0.0 { v=vec3<f32>((vec2<f32>(1.0)-abs(v.yx))*select(vec2<f32>(-1.0),vec2<f32>(1.0),v.xy>=vec2<f32>(0.0)),v.z); }
    return normalize(v);
}

// A pixel's receiver and interpolated vertex normal, as primary_shade_pixel builds them.
struct GiReceiver { receiver:Receiver, normal:vec3<f32>, valid:bool };
fn gi_pixel_receiver(id:vec2<u32>) -> GiReceiver {
    var out:GiReceiver;
    if textureLoad(primary_tiles,id/PRIMARY_TILE,0).x==0u { return out; }
    let pixel=vec2<i32>(id);
    let record=textureLoad(primary_hit,pixel,0);
    if record.x==0u { return out; }
    let hit=primary_decode(record);
    let ta=primary_input(hit.batch,hit.ia,hit.placement);let tb=primary_input(hit.batch,hit.ib,hit.placement);
    let tc=primary_input(hit.batch,hit.ic,hit.placement);
    let a=primary_vertex(hit,ta,hit.ia);let b=primary_vertex(hit,tb,hit.ib);let c=primary_vertex(hit,tc,hit.ic);
    let w=vec3<f32>(1.0-hit.bary.x-hit.bary.y,hit.bary);
    let receiver_world=world_position(receiver_point(ta.position,tb.position,tc.position,w),hit.placement).xyz;
    out.normal=a.normal*w.x+b.normal*w.y+c.normal*w.z;
    out.receiver=primary_receiver(receiver_world,array<vec3<f32>,3>(ta.position,tb.position,tc.position),hit.placement,pixel);
    out.valid=true;
    return out;
}

@compute @workgroup_size(64) fn gi_tile_corners(@builtin(global_invocation_id) gid:vec3<u32>, @builtin(num_workgroups) groups:vec3<u32>) {
    let size=textureDimensions(primary_hit);
    let grid=gi_corner_grid(size);
    let index=gid.x+gid.y*groups.x*64u;
    if index>=grid.x*grid.y { return; }
    let corner=vec2<u32>(index%grid.x,index/grid.x);
    let at=index*GI_CORNER_WORDS;
    var flags=0u;
    let p=min(corner*4u,size-vec2<u32>(1u));
    let r=gi_pixel_receiver(p);
    if r.valid && gi_word(GI_H_ENABLED)!=0u {
        var input:SurfaceInput;
        input.world=r.receiver.world;
        input.normal=r.normal;
        input.receiver=r.receiver;
        gi_prepare(input);
        flags=select(1u,3u,gi_rx_valid);
        var h=array<f32,40>();
        for(var i=0u;i<9u;i+=1u) { h[i*3u]=gi_rx_sky[i].x;h[i*3u+1u]=gi_rx_sky[i].y;h[i*3u+2u]=gi_rx_sky[i].z; }
        for(var i=0u;i<4u;i+=1u) { h[27u+i*3u]=gi_rx_bounce[i].x;h[28u+i*3u]=gi_rx_bounce[i].y;h[29u+i*3u]=gi_rx_bounce[i].z; }
        for(var i=0u;i<20u;i+=1u) { gi_corner_records[at+i]=pack2x16float(vec2<f32>(h[i*2u],h[i*2u+1u])); }
        gi_corner_records[at+20u]=bitcast<u32>(r.receiver.world.x);
        gi_corner_records[at+21u]=bitcast<u32>(r.receiver.world.y);
        gi_corner_records[at+22u]=bitcast<u32>(r.receiver.world.z);
        gi_corner_records[at+23u]=gi_oct(gi_rx_normal);
        gi_corner_records[at+24u]=gi_bin(gi_rx_normal);
    }
    gi_corner_records[at+25u]=flags;
}

// The traced entries' gi_prepare with corner records: the blend where the pixel agrees
// with its tile's four corners, its own lookup otherwise.
fn gi_corner_take(input:SurfaceInput) {
    let size=textureDimensions(primary_hit);
    let grid=gi_corner_grid(size);
    let p=input.pixel;
    let tile=p/4u;
    let f=vec2<f32>(p%4u)/4.0;
    let world=input.receiver.world;
    let n=unit_normal(gi_bin_source(gi_viewed_normal(world,input.receiver.geometric),input.normal));
    let bin=gi_bin(n);
    var at=array<u32,4>();
    var ok=gi_word(GI_H_ENABLED)!=0u;
    var corners=array<vec3<f32>,4>();
    for(var k=0u;k<4u;k+=1u) {
        let c=tile+vec2<u32>(k&1u,k>>1u);
        at[k]=(c.y*grid.x+c.x)*GI_CORNER_WORDS;
        corners[k]=vec3<f32>(bitcast<f32>(gi_corner_records[at[k]+20u]),bitcast<f32>(gi_corner_records[at[k]+21u]),bitcast<f32>(gi_corner_records[at[k]+22u]));
        if gi_corner_records[at[k]+25u]!=3u || gi_corner_records[at[k]+24u]!=bin || dot(gi_unoct(gi_corner_records[at[k]+23u]),n)<GI_TILE_COS { ok=false; }
    }
    let expected=mix(mix(corners[0],corners[1],f.x),mix(corners[2],corners[3],f.x),f.y);
    if !ok || distance(expected,world)>GI_TILE_PLANE*distance(world,lighting.eye) {
        gi_prepare(input);
        return;
    }
    let weight=array<f32,4>((1.0-f.x)*(1.0-f.y),f.x*(1.0-f.y),(1.0-f.x)*f.y,f.x*f.y);
    var h=array<f32,40>();
    for(var k=0u;k<4u;k+=1u) {
        for(var i=0u;i<20u;i+=1u) {
            let pair=unpack2x16float(gi_corner_records[at[k]+i]);
            h[i*2u]+=weight[k]*pair.x;
            h[i*2u+1u]+=weight[k]*pair.y;
        }
    }
    for(var i=0u;i<9u;i+=1u) { gi_rx_sky[i]=vec3<f32>(h[i*3u],h[i*3u+1u],h[i*3u+2u]); }
    for(var i=0u;i<4u;i+=1u) { gi_rx_bounce[i]=vec3<f32>(h[27u+i*3u],h[28u+i*3u],h[29u+i*3u]); }
    gi_rx_normal=n;
    gi_rx_valid=true;
}
// ---- end of GI tile corners ----
