// TP2 two-layer reduction (audit 3.5). The coloured sun map holds one layer per transparent
// draw (its nearest sheet's transmitted light and depth); shading tested every layer with
// four loads per sunlit pixel, 52 layers on Outpost. This pass reduces them, per texel, to
// two layers, which shading reads instead (transparency_shadow.wgsl loops min(N, 2)):
//   layer 0: the nearest sheet toward the sun (its transmitted light and depth);
//   layer 1: the product of every other sheet's light, at the second-nearest depth.
// A receiver in front of every sheet, behind one, or behind all of them gets exactly the
// per-layer product (one or two draws reduce exactly). One between the second and the last
// sheet takes the whole product behind the first, darker than the per-layer test. Cleared
// texels hold (1, 1, 1, 1), full light at the far depth, and reduce as such.
@group(0) @binding(0) var tx_layers:texture_2d_array<f32>;
@group(0) @binding(1) var tx_reduced:texture_storage_2d_array<rgba16float,write>;

@compute @workgroup_size(8,8) fn tx_reduce(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(tx_layers);
    if any(id.xy>=size) { return; }
    let p=vec2<i32>(id.xy);
    var near=vec4<f32>(1.0);
    var rest=vec3<f32>(1.0);
    var second=1.0;
    for(var l=0u;l<textureNumLayers(tx_layers);l+=1u) {
        let t=textureLoad(tx_layers,p,i32(l),0);
        if t.a<near.a {
            // The previous nearest joins the rest; it was nearer than any other, so it is
            // now the second-nearest.
            rest*=near.rgb;
            second=near.a;
            near=t;
        } else {
            rest*=t.rgb;
            second=min(second,t.a);
        }
    }
    textureStore(tx_reduced,p,0,near);
    textureStore(tx_reduced,p,1,vec4<f32>(rest,second));
}
