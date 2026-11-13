// ---- GI tile lookup (src/program.rs GiTiles; RENDERLAB_GI_TILE=0 turns it off) ----
// The camera's cache lookup once per 4x4 pixel tile instead of once per pixel. A compact
// pre-pass (gi_tile_lookups, one thread per tile) decodes the receiver at the tile's inner
// pixel (4i+1, 4j+1, clamped to the image) and runs the usual lookup (gi_prepare), so a
// sixteenth of the pixels pay a lookup's latency. Shading then reads its tile's record
// (gi_tile_take): a pixel that agrees with it (the record a hit with a value, one cache bin,
// normals within GI_TILE_COS, the pixel within GI_TILE_PLANE of the eye distance from the
// record's tangent plane) takes its sums; any other pixel (edges, contacts, bin changes,
// a tile whose inner pixel missed) looks itself up as before. The SH is still evaluated at
// each pixel's own normal, so only the cache's spatial sampling coarsens, to the tile.
// Composed into traced stages only, with the records at group 0 binding 68.
//
// Record, 32 words: 0-19 the 27 sky and 12 bounce SH halves (gi_rx_sky, gi_rx_bounce),
// 20-22 the receiver point, 23 the bin-source normal (octahedral snorm16 x2), 24 its bin,
// 25 flags (1 a hit, 2 the lookup found a value).
@group(0) @binding(68) var<storage,read_write> gi_tile_records:array<u32>;
const GI_TILE_WORDS:u32=32u;
const GI_TILE_COS:f32=0.95;
const GI_TILE_PLANE:f32=0.01;

// The tiles across the image, one per 4x4 pixels.
fn gi_tile_grid(size:vec2<u32>) -> vec2<u32> { return (size+vec2<u32>(3u))/4u; }
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

// Whether two lookups' band-0 coefficients differ by under GI_TILE_VARY of the larger, in
// every channel (and within a small absolute floor, where both are dark).
const GI_TILE_VARY:f32=0.03;
fn gi_tile_close(a:vec3<f32>,b:vec3<f32>)->bool {
    return all(abs(a-b)<=max(GI_TILE_VARY*max(abs(a),abs(b)),vec3<f32>(1e-4)));
}
@compute @workgroup_size(64) fn gi_tile_lookups(@builtin(global_invocation_id) gid:vec3<u32>, @builtin(num_workgroups) groups:vec3<u32>) {
    let size=textureDimensions(primary_hit);
    let grid=gi_tile_grid(size);
    let index=gid.x+gid.y*groups.x*64u;
    if index>=grid.x*grid.y { return; }
    let tile=vec2<u32>(index%grid.x,index/grid.x);
    let at=index*GI_TILE_WORDS;
    var flags=0u;
    // Two lookups at the tile's opposite corners. Where they disagree (a contact, an edge, a
    // steep gradient between cache cells) no pixel takes the tile: one record would step at
    // the tile's 4 pixels. Where they agree the record is their mean.
    let last=size-vec2<u32>(1u);
    let r=gi_pixel_receiver(min(tile*4u,last));
    let q=gi_pixel_receiver(min(tile*4u+vec2<u32>(3u),last));
    if r.valid && q.valid && gi_word(GI_H_ENABLED)!=0u {
        var input:SurfaceInput;
        input.world=q.receiver.world;
        input.normal=q.normal;
        input.receiver=q.receiver;
        gi_prepare(input);
        let q_valid=gi_rx_valid;
        let q_bin=gi_bin(gi_rx_normal);
        let q_sky=gi_rx_sky;
        let q_bounce=gi_rx_bounce;
        input.world=r.receiver.world;
        input.normal=r.normal;
        input.receiver=r.receiver;
        gi_prepare(input);
        let agree=gi_rx_valid && q_valid && q_bin==gi_bin(gi_rx_normal)
            && gi_tile_close(gi_rx_sky[0],q_sky[0]) && gi_tile_close(gi_rx_bounce[0],q_bounce[0]);
        flags=select(1u,3u,agree);
        for(var i=0u;i<9u;i+=1u) { gi_rx_sky[i]=0.5*(gi_rx_sky[i]+q_sky[i]); }
        for(var i=0u;i<4u;i+=1u) { gi_rx_bounce[i]=0.5*(gi_rx_bounce[i]+q_bounce[i]); }
        var h=array<f32,40>();
        for(var i=0u;i<9u;i+=1u) { h[i*3u]=gi_rx_sky[i].x;h[i*3u+1u]=gi_rx_sky[i].y;h[i*3u+2u]=gi_rx_sky[i].z; }
        for(var i=0u;i<4u;i+=1u) { h[27u+i*3u]=gi_rx_bounce[i].x;h[28u+i*3u]=gi_rx_bounce[i].y;h[29u+i*3u]=gi_rx_bounce[i].z; }
        for(var i=0u;i<20u;i+=1u) { gi_tile_records[at+i]=pack2x16float(vec2<f32>(h[i*2u],h[i*2u+1u])); }
        gi_tile_records[at+20u]=bitcast<u32>(r.receiver.world.x);
        gi_tile_records[at+21u]=bitcast<u32>(r.receiver.world.y);
        gi_tile_records[at+22u]=bitcast<u32>(r.receiver.world.z);
        gi_tile_records[at+23u]=gi_oct(gi_rx_normal);
        gi_tile_records[at+24u]=gi_bin(gi_rx_normal);
    }
    gi_tile_records[at+25u]=flags;
}

// The traced entries' gi_prepare with tile records: the tile's sums where the pixel agrees
// with its record, its own lookup otherwise.
fn gi_tile_take(input:SurfaceInput) {
    // Without the cache the records are not written (and may be a placeholder buffer).
    if gi_word(GI_H_ENABLED)==0u { gi_prepare(input); return; }
    let grid=gi_tile_grid(textureDimensions(primary_hit));
    let tile=input.pixel/4u;
    let at=(tile.y*grid.x+tile.x)*GI_TILE_WORDS;
    let world=input.receiver.world;
    let n=unit_normal(gi_bin_source(gi_viewed_normal(world,input.receiver.geometric),input.normal));
    let r=vec3<f32>(bitcast<f32>(gi_tile_records[at+20u]),bitcast<f32>(gi_tile_records[at+21u]),bitcast<f32>(gi_tile_records[at+22u]));
    let rn=gi_unoct(gi_tile_records[at+23u]);
    if gi_tile_records[at+25u]!=3u || gi_tile_records[at+24u]!=gi_bin(n) || dot(rn,n)<GI_TILE_COS
        || abs(dot(world-r,rn))>GI_TILE_PLANE*distance(world,lighting.eye) {
        gi_prepare(input);
        return;
    }
    for(var i=0u;i<9u;i+=1u) {
        let x=i*3u;
        let a=unpack2x16float(gi_tile_records[at+(x>>1u)]);
        let b=unpack2x16float(gi_tile_records[at+((x+1u)>>1u)]);
        let c=unpack2x16float(gi_tile_records[at+((x+2u)>>1u)]);
        gi_rx_sky[i]=vec3<f32>(select(a.x,a.y,(x&1u)!=0u),select(b.x,b.y,((x+1u)&1u)!=0u),select(c.x,c.y,((x+2u)&1u)!=0u));
    }
    for(var i=0u;i<4u;i+=1u) {
        let x=27u+i*3u;
        let a=unpack2x16float(gi_tile_records[at+(x>>1u)]);
        let b=unpack2x16float(gi_tile_records[at+((x+1u)>>1u)]);
        let c=unpack2x16float(gi_tile_records[at+((x+2u)>>1u)]);
        gi_rx_bounce[i]=vec3<f32>(select(a.x,a.y,(x&1u)!=0u),select(b.x,b.y,((x+1u)&1u)!=0u),select(c.x,c.y,((x+2u)&1u)!=0u));
    }
    gi_rx_normal=n;
    gi_rx_valid=true;
}
// ---- end of GI tile lookup ----
