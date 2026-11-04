// Private offline prototype. 8192 GGX + 8192 HDR samples per output, never clipped.
struct Config { base: vec4f, north: vec4f, south: vec4f, extent: vec4f }
struct Query { n: vec4f, t: vec4f, b: vec4f }
struct Hdr { d: vec4f, value: vec4f }
@group(0) @binding(0) var<storage, read> source: array<vec4f>;
@group(0) @binding(1) var<storage, read> hdr: array<Hdr>;
@group(0) @binding(2) var<storage, read> ggx: array<vec4f>;
@group(0) @binding(3) var<storage, read> queries: array<Query>;
@group(0) @binding(4) var<storage, read_write> output: array<vec4f>;
@group(0) @binding(5) var<uniform> cfg: Config;
@group(0) @binding(6) var<uniform> dispatch: vec4u;
var<workgroup> sums: array<vec3f,128>;

fn pixel(x: i32, y: i32) -> vec4f {
    let w=i32(cfg.extent.x); let h=i32(cfg.extent.y);
    return source[u32(clamp(y,0,h-1)*w+((x%w)+w)%w)];
}
fn source_query(d:vec3f) -> vec4f {
    let uv=vec2f(atan2(d.z,d.x)*0.15915494309189535+0.5,acos(clamp(d.y,-1.,1.))*0.3183098861837907);
    let xy=uv*cfg.extent.xy-0.5; let ij=vec2i(floor(xy)); let f=fract(xy);
    var rgb=mix(mix(pixel(ij.x,ij.y).xyz,pixel(ij.x+1,ij.y).xyz,f.x),mix(pixel(ij.x,ij.y+1).xyz,pixel(ij.x+1,ij.y+1).xyz,f.x),f.y);
    if xy.y<0. { rgb=mix(rgb,cfg.north.xyz,-2.*xy.y); }
    if xy.y>cfg.extent.y-1. { rgb=mix(rgb,cfg.south.xyz,2.*(xy.y-cfg.extent.y+1.)); }
    let bin=vec2i(floor(uv*cfg.extent.xy));
    return vec4f(rgb,pixel(bin.x,bin.y).w);
}

@compute @workgroup_size(128)
fn main(@builtin(workgroup_id) group:vec3u,@builtin(local_invocation_index) lane:u32) {
    let index=dispatch.x+group.x;
    let query=queries[index];
    var sum=vec3f(0.); var correction=vec3f(0.);
    for(var j=lane;j<8192u;j+=128u) {
        let g=ggx[u32(query.b.w)+j];
        var a=vec3f(0.);
        if g.z>0. {
            let d=g.x*query.t.xyz+g.y*query.b.xyz+g.z*query.n.xyz;
            let s=source_query(d);
            a=(s.xyz-cfg.base.xyz)*(g.w*g.z/(8192.*(g.w+s.w)));
        }
        let h=hdr[j];
        // |d-n|^2/2 is 1-dot(d,n) for unit vectors without subtracting near-one floats.
        let delta=h.d.xyz-query.n.xyz;
        let half_distance=dot(delta,delta)*0.25;
        let cosine=1.-2.*half_distance;
        if cosine>0. {
            let a2=query.t.w;
            let denominator=a2+(1.-a2)*half_distance;
            let pg=a2/(12.566370614359172*denominator*denominator);
            a+=(h.value.xyz-cfg.base.xyz)*(pg*cosine/(8192.*(pg+h.d.w)));
        }
        let corrected=a-correction;
        let next=sum+corrected;
        correction=(next-sum)-corrected;
        sum=next;
    }
    sums[lane]=sum;
    workgroupBarrier();
    for(var stride=64u;stride>0u;stride/=2u) {
        if lane<stride { sums[lane]+=sums[lane+stride]; }
        workgroupBarrier();
    }
    if lane==0u { output[index]=vec4f(cfg.base.xyz+sums[0]*query.n.w,16384.); }
}
