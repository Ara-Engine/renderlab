// ---- renderlab shading-program API, traced texture-residency sampling ----
// Replaces the traced API's `sample_*_grad` helpers in the shading pipelines of frames
// that stream textures (src/texture_residency.rs, program.rs Stage::TracedStreamed).
// Default frames compose the traced API unchanged and never contain this code.
// Texture residency (src/texture_residency.rs). Per slot and role: the residency word
// (zero for a fully resident texture); then per bucket of 8x8 screen tiles, three
// per-frame counters: 16 minus the finest level any sample requested, the samples that
// requested a level below the resident top, and 16 minus the finest touched level
// (survey).
// The helpers below record them, so every program's samples are covered.
struct PrimaryResidency { slots:array<u32,RL_SLOTS>, counters:array<atomic<u32>> };
const RL_SLOTS:u32=3u*PRIMARY_BATCHES;
// Counter copies spread one frame's same-word traffic over separate cache lines.
fn rl_bucket(pixel:vec2<u32>)->u32 { return (((pixel.x>>3u)+(pixel.y>>3u)*5u)&31u)*3u*RL_SLOTS; }
@group(2) @binding(18) var<storage,read_write> primary_residency:PrimaryResidency;
// Level j holds 2^-j. Sampled with a material's own sampler and the footprint scaled to
// it, its blend's finer level is the finest level that material sample touches.
@group(2) @binding(19) var primary_level_probe:texture_2d<f32>;
const RL_PROBED:u32=0x40000000u;
const RL_SURVEY:u32=0x20000000u;
// Whether this sample is probed. Probed slots request the touched level less their
// margin (bits 14-15: anisotropic filtering of a tail differs at its base level, see
// texture_residency.rs); others request every level and are probed only when surveyed.
fn rl_probed(word:u32)->bool { return (word&(RL_PROBED|RL_SURVEY))!=0u; }
// Footprint scale to the probe, 2^(field-16) per axis, built exactly from exponent bits.
fn rl_scale(word:u32)->vec2<f32> {
    return vec2<f32>(bitcast<f32>((((word>>4u)&31u)+111u)<<23u),bitcast<f32>((((word>>9u)&31u)+111u)<<23u));
}
fn rl_code(index:u32, code:u32) {
    if code>atomicLoad(&primary_residency.counters[index]) {atomicMax(&primary_residency.counters[index],code);}
}
// A blend of probe levels j and j+1 lies in (2^-(j+1), 2^-j]: floor(-log2) is the finer.
fn rl_record(pixel:vec2<u32>, slot:u32, word:u32, probe:f32) {
    let index=rl_bucket(pixel)+slot;
    let probed=u32(clamp(floor(0.000244140625-log2(probe)),0.0,15.0));
    let level=select(0u,probed-min(probed,(word>>14u)&3u),(word&RL_PROBED)!=0u);
    rl_code(index,16u-level);
    if level<(word&15u) {atomicAdd(&primary_residency.counters[RL_SLOTS+index],1u);}
    if (word&RL_SURVEY)!=0u {rl_code(2u*RL_SLOTS+index,16u-probed);}
}
fn sample_base_color_grad(input:SurfaceInput, uv:vec2<f32>, dx:vec2<f32>, dy:vec2<f32>) -> vec4<f32> {
    if !rl_term(RL_TERM_TEXTURES) { return vec4<f32>(1.0); }
    let i=input.receiver.slot;
    let word=primary_residency.slots[3u*i];
    if word!=0u {
        var probe=1.0;
        if rl_probed(word) {
            let s=rl_scale(word);
            probe=textureSampleGrad(primary_level_probe,primary_base_sampler[i],uv,dx*s,dy*s).r;
        }
        rl_record(input.pixel,3u*i,word,probe);
    }
    return textureSampleGrad(primary_base_color[i],primary_base_sampler[i],uv,dx,dy);
}
fn sample_metallic_roughness_grad(input:SurfaceInput, uv:vec2<f32>, dx:vec2<f32>, dy:vec2<f32>) -> vec4<f32> {
    let i=input.receiver.slot;
    let word=primary_residency.slots[3u*i+1u];
    if word!=0u {
        var probe=1.0;
        if rl_probed(word) {
            let s=rl_scale(word);
            probe=textureSampleGrad(primary_level_probe,primary_mr_sampler[i],uv,dx*s,dy*s).r;
        }
        rl_record(input.pixel,3u*i+1u,word,probe);
    }
    return textureSampleGrad(primary_metallic_roughness[i],primary_mr_sampler[i],uv,dx,dy);
}
fn sample_normal_layer_grad(input:SurfaceInput, uv:vec2<f32>, dx:vec2<f32>, dy:vec2<f32>, layer:i32) -> vec4<f32> {
    if !rl_term(RL_TERM_TEXTURES) { return vec4<f32>(0.5,0.5,1.0,0.0); }
    let i=input.receiver.slot;
    let word=primary_residency.slots[3u*i+2u];
    if word!=0u {
        var probe=1.0;
        if rl_probed(word) {
            let s=rl_scale(word);
            probe=textureSampleGrad(primary_level_probe,primary_normal_sampler[i],uv,dx*s,dy*s).r;
        }
        rl_record(input.pixel,3u*i+2u,word,probe);
    }
    return textureSampleGrad(primary_normal_map[i],primary_normal_sampler[i],uv,layer,dx,dy);
}
