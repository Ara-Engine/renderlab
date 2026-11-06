// Test-only conditioned visibility of T1's own samples (src/film_accumulation_tests.rs).
// After each temporal frame, a per-sample shading pass writes each pixel's jittered sample
// as (visibility, depth along the camera forward in two parts); this accumulates the visibility of the
// samples on the pixel's centre surface (same placement, depth within 2%), the estimand of
// the film reference's R_F.
struct Centre { placement:u32, depth:f32 };
@group(0) @binding(0) var hit:texture_2d<u32>;
@group(0) @binding(1) var hdr:texture_2d<f32>;
@group(0) @binding(2) var tiles:texture_2d<u32>;
@group(0) @binding(3) var<storage,read> centre:array<Centre>;
@group(0) @binding(4) var<storage,read_write> result:array<vec2<u32>>;
@compute @workgroup_size(8,4) fn accumulate(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(hit);
    if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);let i=id.x+id.y*size.x;
    if textureLoad(tiles,id.xy/vec2<u32>(8u,4u),0).x==0u {return;}
    let c=centre[i];let h=textureLoad(hit,p,0).x;
    let sample=textureLoad(hdr,p,0);
    // Depth arrives as an f16 high part plus its residual biased by high/1024
    // (film_accumulation_tests.rs rl_conditioned).
    let depth=sample.y+sample.z-sample.y/1024.0;
    if h!=0u && h==c.placement && abs(depth-c.depth)<=0.02*c.depth {
        result[i]+=vec2<u32>(u32(round(sample.x*256.0)),1u);
    }
}
