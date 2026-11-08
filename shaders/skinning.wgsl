// SK1: material-independent linear blend skinning, before visibility, AS and shadows.
struct Influence { joints:vec4<u32>, weights:vec4<f32> };
struct Params { vertices:u32, mappings:u32, tangents:u32, first:u32 };
@group(0) @binding(0) var<storage,read> source:array<f32>;
@group(0) @binding(1) var<storage,read> influences:array<Influence>;
@group(0) @binding(2) var<storage,read> palette:array<mat4x4<f32>>;
@group(0) @binding(3) var<storage,read_write> vertices:array<f32>;
@group(0) @binding(4) var<storage,read_write> previous:array<f32>;
@group(0) @binding(5) var<uniform> params:Params;
@group(0) @binding(6) var<storage,read> tangent_source:array<vec4<f32>>;
@group(0) @binding(7) var<storage,read_write> tangents:array<vec4<f32>>;
@group(0) @binding(8) var<storage,read> mappings:array<vec2<u32>>;
@group(0) @binding(9) var<storage,read_write> cluster_data:array<u32>;
fn safe_normal(v:vec3<f32>,fallback:vec3<f32>)->vec3<f32> {
    let l=dot(v,v);if l>1e-20 {return v*inverseSqrt(l);}return fallback;
}
@compute @workgroup_size(64) fn skin_vertices(@builtin(global_invocation_id) id:vec3<u32>) {
    let i=id.x;if i>=params.vertices {return;}
    let o=i*12u;let w=influences[i];
    let m=palette[w.joints.x]*w.weights.x+palette[w.joints.y]*w.weights.y+palette[w.joints.z]*w.weights.z+palette[w.joints.w]*w.weights.w;
    let p=(m*vec4<f32>(source[o],source[o+1u],source[o+2u],1.0)).xyz;
    let n0=vec3<f32>(source[o+3u],source[o+4u],source[o+5u]);
    let a=m[0].xyz;let b=m[1].xyz;let c=m[2].xyz;
    let cof=mat3x3<f32>(cross(b,c),cross(c,a),cross(a,b));
    let determinant=dot(a,cross(b,c));
    let n=safe_normal((cof*n0)*select(-1.0,1.0,determinant>=0.0),n0);
    for(var j=0u;j<12u;j+=1u) {previous[o+j]=vertices[o+j];vertices[o+j]=source[o+j];}
    vertices[o]=p.x;vertices[o+1u]=p.y;vertices[o+2u]=p.z;
    vertices[o+3u]=n.x;vertices[o+4u]=n.y;vertices[o+5u]=n.z;
    if params.first!=0u {for(var j=0u;j<12u;j+=1u) {previous[o+j]=vertices[o+j];}}
    if params.tangents!=0u {
        let t=tangent_source[i];let q=mat3x3<f32>(a,b,c)*t.xyz;
        tangents[i]=vec4<f32>(safe_normal(q-n*dot(q,n),t.xyz),t.w*select(-1.0,1.0,determinant>=0.0));
    }
}
// A separate dispatch/pass follows the vertex writes. Full-detail source clusters
// retain their original topology and primitive IDs; only position copies change.
@compute @workgroup_size(64) fn skin_clusters(@builtin(global_invocation_id) id:vec3<u32>) {
    let i=id.x;if i>=params.mappings {return;}
    let m=mappings[i];let o=m.y*12u;
    cluster_data[m.x]=bitcast<u32>(vertices[o]);cluster_data[m.x+1u]=bitcast<u32>(vertices[o+1u]);cluster_data[m.x+2u]=bitcast<u32>(vertices[o+2u]);
}
