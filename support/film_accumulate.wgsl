// Test-only F0 accumulator. Visibility is conditioned on the centre surface;
// radiance includes every primary sample, including background and silhouettes.
struct Pixel { counts:vec4<u32>, moments:vec4<f32>, radiance:vec4<f32> };
struct Params { eye:vec4<f32>, forward:vec4<f32>, index:vec4<u32> };
@group(0) @binding(0) var hit:texture_2d<u32>;
@group(0) @binding(1) var world:texture_2d<f32>;
@group(0) @binding(2) var visibility:texture_2d<u32>;
@group(0) @binding(3) var hdr:texture_2d<f32>;
@group(0) @binding(4) var<storage,read> centre:array<vec4<f32>>;
@group(0) @binding(5) var<storage,read_write> result:array<Pixel>;
@group(0) @binding(6) var<uniform> params:Params;
@compute @workgroup_size(8,4) fn accumulate(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(hit);
    if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);let i=id.x+id.y*size.x;
    let c=centre[i];let h=textureLoad(hit,p,0).x;
    let point=textureLoad(world,p,0).xyz;
    let dc=dot(c.xyz-params.eye.xyz,params.forward.xyz);
    let ds=dot(point-params.eye.xyz,params.forward.xyz);
    if h!=0u && h==bitcast<u32>(c.w) && abs(ds-dc)<=0.02*dc {
        let v=textureLoad(visibility,p,0).x;
        let half=params.index.x%2u;
        result[i].counts[half]+=v;
        result[i].counts[2u+half]+=1u;
        result[i].moments.x+=f32(v*v)/65536.0;
    }
    result[i].radiance+=textureLoad(hdr,p,0);
}
