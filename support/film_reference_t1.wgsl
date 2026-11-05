// Test-only T1 film reference accumulator (src/film_accumulation_tests.rs): 1,024 jittered
// primary samples in two independent stratified halves of 512 (index / 512), so scores can
// subtract reference noise. Visibility is conditioned on the centre surface, as in F0/R1;
// radiance includes every primary sample, including background and silhouettes.
struct Pixel { counts:vec4<u32>, moments:vec4<f32>, radiance0:vec4<f32>, radiance1:vec4<f32> };
struct Params { eye:vec4<f32>, forward:vec4<f32>, index:vec4<u32> };
@group(0) @binding(0) var hit:texture_2d<u32>;
@group(0) @binding(1) var world:texture_2d<f32>;
@group(0) @binding(2) var visibility:texture_2d<u32>;
@group(0) @binding(3) var hdr:texture_2d<f32>;
@group(0) @binding(4) var<storage,read> centre:array<vec4<f32>>;
@group(0) @binding(5) var<storage,read_write> result:array<Pixel>;
@group(0) @binding(6) var<uniform> params:Params;
@group(0) @binding(8) var tiles:texture_2d<u32>;
@compute @workgroup_size(8,4) fn accumulate(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(hit);
    if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);let i=id.x+id.y*size.x;
    let half=params.index.x/512u;
    // Uncovered trace tiles leave stale hit records and receivers.
    if textureLoad(tiles,id.xy/vec2<u32>(8u,4u),0).x!=0u {
        let c=centre[i];let h=textureLoad(hit,p,0).x;
        let point=textureLoad(world,p,0).xyz;
        let dc=dot(c.xyz-params.eye.xyz,params.forward.xyz);
        let ds=dot(point-params.eye.xyz,params.forward.xyz);
        if h!=0u && h==bitcast<u32>(c.w) && abs(ds-dc)<=0.02*dc {
            let v=textureLoad(visibility,p,0).x;
            result[i].counts[half]+=v;
            result[i].counts[2u+half]+=1u;
            result[i].moments[half]+=f32(v*v)/65536.0;
        }
    }
    let x=textureLoad(hdr,p,0);
    if half==0u {result[i].radiance0+=x;} else {result[i].radiance1+=x;}
}
