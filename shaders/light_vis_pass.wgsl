// Exact visibility for the first 16 shadowed local lights of each pixel's cluster.
// Row-major pixels, four packed u32 words per pixel (16 bytes); overflow stays inline.
// Each byte is round(clamp(visibility,0,1)*255); overflow remains inline.
@group(0) @binding(73) var<storage,read_write> rl_lvis:array<u32>;
const RL_LVIS_SLOTS:u32=16u;
const RL_LVIS_FACING:bool=false;
// Reserved only with the facing gate. A natural 254 also falls back inline, so
// all 256 unorm values remain representable without another per-pixel record.
const RL_LVIS_UNTRACED:u32=254u;
var<private> rl_lvis_ready:bool;
fn rl_lvis_lookup(input:SurfaceInput, s:LightSample)->f32 {
    if !rl_lvis_ready { return -1.0; }
    let base=(input.pixel.y*textureDimensions(primary_hit).x+input.pixel.x)*4u;
    if base+3u>=arrayLength(&rl_lvis) { return -1.0; }
    var slot=0u;
    for(var i=0u;i<rl_light_total();i+=1u) {
        let index=rl_light_index(i);
        if lights[index].kind.w==0u { continue; }
        if slot>=RL_LVIS_SLOTS { break; }
        if index==s.index {
            let value=(rl_lvis[base+(slot>>2u)]>>((slot&3u)*8u))&255u;
            if RL_LVIS_FACING && value==RL_LVIS_UNTRACED { return -1.0; }
            return f32(value)*(1.0/255.0);
        }
        slot+=1u;
    }
    return -1.0;
}
fn rl_lvis_store(input:SurfaceInput) {
    rl_lights_prepare(input.world);
    let base=(input.pixel.y*textureDimensions(primary_hit).x+input.pixel.x)*4u;
    if base+3u>=arrayLength(&rl_lvis) { return; }
    var packed=0u;
    var slot=0u;
    for(var i=0u;i<rl_light_total();i+=1u) {
        let index=rl_light_index(i);
        if lights[index].kind.w==0u { continue; }
        if slot>=RL_LVIS_SLOTS { break; }
        let sample=light_sample_untinted(i,input);
        var value=RL_LVIS_UNTRACED;
        // This is only a producer work gate, not a material-facing decision. A
        // normal map or a custom shader can still need this light: lookup then
        // returns -1 and runs the original visibility service inline. Thin and
        // two-sided receivers always retain the producer's rays.
        if !RL_LVIS_FACING || input.receiver.thin || input.material.thin!=0u || input.material.double_sided!=0u
            || dot(input.geometric,sample.direction)>0.0 {
            let visibility=light_visibility(input,sample);
            value=u32(round(clamp(visibility,0.0,1.0)*255.0));
        }
        packed|=value<<((slot&3u)*8u);
        if (slot&3u)==3u { rl_lvis[base+(slot>>2u)]=packed; packed=0u; }
        slot+=1u;
    }
    if (slot&3u)!=0u { rl_lvis[base+(slot>>2u)]=packed; }
}
@compute @workgroup_size(8,4)
fn light_vis_tiles(@builtin(global_invocation_id) grid:vec3<u32>) {
    let id=grid.xy+primary_region.xy;
    let size=textureDimensions(primary_hit);
    if any(id>=size) || any(id>=primary_region.zw) { return; }
    if textureLoad(primary_tiles,id/PRIMARY_TILE,0).x==0u { return; }
    rl_lvis_pixel(id,size);
}
