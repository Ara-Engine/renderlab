// Software raster of small clusters entirely in front of the near plane: one workgroup
// per cluster, its at most 128 vertices projected once into workgroup memory, then one
// invocation per triangle. Vertices snap to the hardware's 8 subpixel bits and edge
// functions are exact integers, so shared edges cannot crack between software triangles.
// The inclusive test covers pixel centres on an edge from both sides; the 64-bit max
// keeps the nearer (then larger-ID) triangle and resolve's edge repair classifies ties.
// Software clusters from 0 (visibility_buffer_cull.wgsl).
@group(0) @binding(3) var<storage,read> lists:array<u32>;
struct Args {sw_x:u32,sw_y:u32,sw_z:u32,sw_count:u32};
@group(0) @binding(5) var<storage,read> args:Args;
@group(0) @binding(8) var<storage,read_write> visibility:array<atomic<u64>>;
// Its own group: wgpu forbids binding arrays beside uniform buffers.
@group(2) @binding(8) var<storage,read> cluster_data:binding_array<U32s,PRIMARY_BATCHES>;
// Pixel x, y (y down) and NDC depth.
var<workgroup> screen:array<vec3<f32>,128>;
var<workgroup> wg_cluster:u32;
fn edge(a:vec2<i32>,b:vec2<i32>,p:vec2<i32>)->i32 {return (b.x-a.x)*(p.y-a.y)-(b.y-a.y)*(p.x-a.x);}
@compute @workgroup_size(128) fn raster(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    let slot=group.x+group.y*65535u;
    if li==0u {wg_cluster=select(0xffffffffu,lists[slot],slot<args.sw_count);}
    let index=workgroupUniformLoad(&wg_cluster);
    if index==0xffffffffu {return;}
    let c=clusters[index];
    let size=image_size();let fsize=vec2<f32>(size);
    let packed=placements[c.placement];let batch=packed&0x1fffffffu;
    if li<((c.counts>>8u)&255u) {
        let o=c.vertices+3u*li;let d=&cluster_data[batch].data;
        let p=vec3<f32>(bitcast<f32>((*d)[o]),bitcast<f32>((*d)[o+1u]),bitcast<f32>((*d)[o+2u]));
        // The hardware path's association, VP*(model*p).
        let clip=config.matrix*(models[c.placement]*vec4<f32>(p,1.0));
        let ndc=clip.xyz/clip.w;
        screen[li]=vec3<f32>((ndc.x*0.5+0.5)*fsize.x,(0.5-ndc.y*0.5)*fsize.y,ndc.z);
    }
    workgroupBarrier();
    if li>=(c.counts&255u) {return;}
    let t=cluster_data[batch].data[c.triangles+li];
    let s0=screen[t&255u];let s1=screen[(t>>8u)&255u];let s2=screen[(t>>16u)&255u];
    let a=vec2<i32>(round(s0.xy*256.0));var b=vec2<i32>(round(s1.xy*256.0));var d=vec2<i32>(round(s2.xy*256.0));
    var zb=s1.z;var zd=s2.z;
    let area=edge(a,b,d);
    if area==0 {return;}
    // Counter-clockwise NDC (front, wgpu's default) is negative area with y down.
    let front=area<0;
    if (packed&0x40000000u)!=0u && front!=(normals[c.placement].info.x>0.0) {return;}
    if area<0 {let p=b;b=d;d=p;let z=zb;zb=zd;zd=z;}
    let inverse=1.0/f32(abs(area));
    // Pixel centres are at 256x+128 subpixels.
    let lo=max(min(a,min(b,d)),vec2<i32>(0));
    let hi=min(max(a,max(b,d)),vec2<i32>(size)*256-vec2<i32>(1));
    let first=(lo-vec2<i32>(128)+vec2<i32>(255))>>vec2<u32>(8u);
    let last=(hi-vec2<i32>(128))>>vec2<u32>(8u);
    let payload=u64((index<<7u)|li);
    for(var y=first.y;y<=last.y;y+=1) {
        for(var x=first.x;x<=last.x;x+=1) {
            let p=vec2<i32>(x*256+128,y*256+128);
            let w0=edge(b,d,p);let w1=edge(d,a,p);let w2=edge(a,b,p);
            if (w0|w1|w2)<0 {continue;}
            let z=(f32(w0)*s0.z+f32(w1)*zb+f32(w2)*zd)*inverse;
            if z<=0.0 || z>1.0 {continue;}
            atomicMax(&visibility[u32(y)*size.x+u32(x)],(u64(bitcast<u32>(z))<<32u)|payload);
        }
    }
}
