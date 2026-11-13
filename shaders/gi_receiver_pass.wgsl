// GI receiver lookup output: 48 bytes/pixel, 2 f32 scales + 39 snorm8 coefficients.
// Sky and bounce stay separate so the shading stage preserves their separate clamps.
struct GiPrepared { scales:vec2<f32>, packed:array<u32,10> };
@group(0) @binding(68) var<storage,read_write> gi_prepared:array<GiPrepared>;
fn gi_pass_component(i:u32) -> f32 {
    if i<27u { return gi_rx_sky[i/3u][i%3u]; }
    if i<39u { return gi_rx_bounce[(i-27u)/3u][(i-27u)%3u]; }
    return 0.0;
}
fn gi_pass_load(input:SurfaceInput,id:vec2<u32>,size:vec2<u32>) {
    let at=id.y*size.x+id.x;
    let scales=gi_prepared[at].scales;
    gi_rx_valid=scales.x>=0.0;
    gi_rx_normal=unit_normal(gi_bin_source(gi_viewed_normal(input.receiver.world,input.receiver.geometric),input.normal));
    for(var i=0u;i<39u;i+=1u) {
        let value=unpack4x8snorm(gi_prepared[at].packed[i/4u])[i%4u]*select(scales.x,scales.y,i>=27u);
        // Dynamic vector stores are unreliable on NVIDIA: construct whole RGB vectors below.
        if i%3u==0u {
            let b=i+1u;let c=i+2u;
            let y=unpack4x8snorm(gi_prepared[at].packed[b/4u])[b%4u]*select(scales.x,scales.y,b>=27u);
            let z=unpack4x8snorm(gi_prepared[at].packed[c/4u])[c%4u]*select(scales.x,scales.y,c>=27u);
            if i<27u { gi_rx_sky[i/3u]=vec3<f32>(value,y,z); }
            else { gi_rx_bounce[(i-27u)/3u]=vec3<f32>(value,y,z); }
        }
    }
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
    for(var i=0u;i<39u;i+=1u) {
        if i<27u {sky=max(sky,abs(gi_pass_component(i)));}
        else {bounce=max(bounce,abs(gi_pass_component(i)));}
    }
    gi_prepared[at].scales=vec2<f32>(select(-1.0,sky,gi_rx_valid),bounce);
    for(var k=0u;k<10u;k+=1u) {
        let i=k*4u;
        let scales=vec4<f32>(select(sky,bounce,i>=27u),select(sky,bounce,i+1u>=27u),select(sky,bounce,i+2u>=27u),select(sky,bounce,i+3u>=27u));
        let values=vec4<f32>(gi_pass_component(i),gi_pass_component(i+1u),gi_pass_component(i+2u),gi_pass_component(i+3u));
        gi_prepared[at].packed[k]=pack4x8snorm(values/max(scales,vec4<f32>(1e-30)));
    }
}
