// LC1 clustered light culling (docs/m7-lc1.md). One thread per cluster of the camera's
// froxel grid (src/light_cluster.rs prepends the LC_* constants and the LcFrame struct):
// LC_X x LC_Y screen tiles times LC_Z depth slices, logarithmic in view depth from the near
// plane. Each thread tests every light's bounding sphere against its cluster's view-space
// box and writes the indices it meets, in light order, as u16 pairs. Directional and
// unlimited-range lights meet every cluster. The count word keeps the stored count in its
// low 16 bits and the count the cluster wanted (before LC_MAX truncation) in its high 16.
// Lists live in an R32Uint texture (no storage buffer in the shading stages): cluster c's
// word k is texel (c % LC_ROW * LC_STRIDE + k, c / LC_ROW).
struct Light { position_range:vec4<f32>, travel_outer:vec4<f32>, rgb_inner:vec4<f32>, kind:vec4<u32>, source:vec4<f32> };
@group(0) @binding(0) var<storage,read> lights:array<Light>;
@group(0) @binding(1) var clusters:texture_storage_2d<r32uint,write>;
@group(0) @binding(2) var<uniform> frame:LcFrame;
var<workgroup> lc_spheres:array<vec4<f32>,64>;

fn lc_store(c:u32, k:u32, value:u32) {
    textureStore(clusters,vec2<u32>((c%LC_ROW)*LC_STRIDE+k,c/LC_ROW),vec4<u32>(value,0u,0u,0u));
}
// A light's bounding sphere in view space; w < 0 marks a light that meets every cluster.
fn lc_sphere(light:Light) -> vec4<f32> {
    let range=light.position_range.w;
    if light.kind.x==0u || range<=0.0 { return vec4<f32>(0.0,0.0,0.0,-1.0); }
    var centre=light.position_range.xyz;
    var radius=range;
    if light.kind.x==2u {
        // The cone's bounding sphere: the cap's circumsphere for narrow cones, the
        // cap disc's for wide ones.
        let c=clamp(light.travel_outer.w,0.0,1.0);
        let s=sqrt(max(1.0-c*c,0.0));
        let axis=light.travel_outer.xyz;
        if c>0.70710678 {
            radius=range/(2.0*c);
            centre+=axis*radius;
        } else {
            radius=range*s;
            centre+=axis*(range*c);
        }
    }
    // Pad for the f32 cluster assignment at box faces; light beyond its range is zero.
    return vec4<f32>((frame.view*vec4<f32>(centre,1.0)).xyz,radius*1.001+1e-4);
}
fn lc_depth(slice:u32) -> f32 {
    if slice>=LC_Z { return frame.far.x; }
    return frame.proj.z*exp2(f32(slice)/frame.proj.w);
}
@compute @workgroup_size(64) fn lc_build(@builtin(global_invocation_id) id:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let c=id.x;
    let inside=c<LC_CLUSTERS;
    let tx=c%LC_X;let ty=(c/LC_X)%LC_Y;let tz=c/(LC_X*LC_Y);
    // The tile's NDC rectangle (y down in tiles) and the slice's depth interval.
    let x0=-1.0+2.0*f32(tx)/f32(LC_X);let x1=-1.0+2.0*f32(tx+1u)/f32(LC_X);
    let y1=1.0-2.0*f32(ty)/f32(LC_Y);let y0=1.0-2.0*f32(ty+1u)/f32(LC_Y);
    let d0=lc_depth(tz);
    let d1=select(lc_depth(tz+1u),frame.far.x,tz+1u>=LC_Z);
    // View x = ndc x * depth / proj x: the box spans both depths' corners.
    let ax=vec4<f32>(x0*d0,x0*d1,x1*d0,x1*d1)/frame.proj.x;
    let ay=vec4<f32>(y0*d0,y0*d1,y1*d0,y1*d1)/frame.proj.y;
    let lo=vec3<f32>(min(min(ax.x,ax.y),min(ax.z,ax.w)),min(min(ay.x,ay.y),min(ay.z,ay.w)),-d1);
    let hi=vec3<f32>(max(max(ax.x,ax.y),max(ax.z,ax.w)),max(max(ay.x,ay.y),max(ay.z,ay.w)),-d0);
    var stored=0u;
    var wanted=0u;
    var pair=0u;
    let count=frame.info.x;
    for(var first=0u;first<count;first+=64u) {
        workgroupBarrier();
        if first+lane<count { lc_spheres[lane]=lc_sphere(lights[first+lane]); }
        workgroupBarrier();
        // No early `continue` here: the barriers above must stay in uniform control flow.
        if inside {
            let n=min(64u,count-first);
            for(var k=0u;k<n;k+=1u) {
                let s=lc_spheres[k];
                var hit=s.w<0.0;
                if !hit {
                    let d=s.xyz-clamp(s.xyz,lo,hi);
                    hit=dot(d,d)<=s.w*s.w;
                }
                if !hit { continue; }
                wanted+=1u;
                if stored>=LC_MAX { continue; }
                let index=first+k;
                if (stored&1u)==0u {
                    pair=index;
                } else {
                    lc_store(c,1u+(stored>>1u),pair|(index<<16u));
                }
                stored+=1u;
            }
        }
    }
    if !inside { return; }
    if (stored&1u)==1u { lc_store(c,1u+(stored>>1u),pair); }
    lc_store(c,0u,stored|(min(wanted,0xffffu)<<16u));
}
