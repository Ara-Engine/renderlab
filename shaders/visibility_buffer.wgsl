// N1c: clusterLOD cut, two-phase HZB culling and a software/hardware ID raster into one
// 64-bit (depth, cluster, triangle) visibility buffer. Original implementation of the
// Nanite/Bevy design surveyed in docs/research/reuse-non-rt.md; no third-party source.
// Hardware clusters clip against the near plane; software clusters are small and in front.
enable wgpu_binding_array;
struct Config {
    // Current and previous-frame view-projection; pass 1 tests last frame's HZB.
    matrix:mat4x4<f32>,
    previous:mat4x4<f32>,
    // Camera position; w = pixels per unit error at unit distance (height/2 * cot(fov/2)).
    eye:vec4<f32>,
    // x error threshold in pixels (negative: original geometry only), y near distance,
    // z largest software-raster cluster extent in pixels.
    lod:vec4<f32>,
    // x cluster count, y previous HZB valid, z pass, w width | height << 16.
    info:vec4<u32>,
};
// Object-space box; vertices/triangles are word offsets into the mesh's cluster data
// (three f32 position words per local vertex, then packed u8 local triangles in the
// index buffer's corner order). primitives is the first primitive ID, or with TABLE set
// the offset of per-triangle source primitive IDs. counts holds
// triangles | vertices << 8 | TABLE.
struct Cluster {lo:vec3<f32>,vertices:u32,hi:vec3<f32>,triangles:u32,owner:u32,refined:u32,primitives:u32,counts:u32,placement:u32,pad0:u32,pad1:u32,pad2:u32};
@group(0) @binding(0) var<uniform> config:Config;
@group(0) @binding(1) var<storage,read> clusters:array<Cluster>;
struct U32s {data:array<u32>};
@group(1) @binding(0) var<storage,read> models:array<mat4x4<f32>>;
struct NormalTransform {x:vec4<f32>,y:vec4<f32>,z:vec4<f32>,info:vec4<f32>};
@group(1) @binding(1) var<storage,read> normals:array<NormalTransform>;
@group(1) @binding(5) var<storage,read> placements:array<u32>;

fn image_size()->vec2<u32> {return vec2<u32>(config.info.w&65535u,config.info.w>>16u);}

// Conservative AABB projection: boxes crossing near/eye planes return the whole screen
// with hi.z = 1 and are never occluded or software rasterized.
fn cluster_box(c:Cluster,vp:mat4x4<f32>)->mat2x4<f32> {
    let model=models[c.placement];let m=vp*model;
    // Bound both matrix association orders: raster uses VP*(model*p), while the
    // box uses (VP*model)*corner. Absolute products bound cancellation as well as
    // transformed source points between the corners. 64 eps exceeds the sum of
    // the dot-product rounding bounds; a fixed NDC depth guard hid fine occlusion.
    let pabs=max(abs(c.lo),abs(c.hi));
    let world_abs=abs(model[0])*pabs.x+abs(model[1])*pabs.y+abs(model[2])*pabs.z+abs(model[3]);
    let clip_abs=abs(vp[0])*world_abs.x+abs(vp[1])*world_abs.y+abs(vp[2])*world_abs.z+abs(vp[3])*world_abs.w;
    let error=clip_abs*(64.0*1.1920928955078125e-7);
    var lo=vec4<f32>(1e30);var hi=vec4<f32>(-1e30);
    for(var i=0u;i<8u;i+=1u) {
        let p=vec3<f32>(select(c.lo.x,c.hi.x,(i&1u)!=0u),select(c.lo.y,c.hi.y,(i&2u)!=0u),select(c.lo.z,c.hi.z,(i&4u)!=0u));
        let clip=m*vec4<f32>(p,1.0);
        if clip.w<=error.w || clip.z+error.z>=clip.w-error.w {return mat2x4<f32>(vec4<f32>(-1.0,-1.0,0.0,1.0),vec4<f32>(1.0,1.0,1.0,1.0));}
        let q=clip.xyz/clip.w;
        let margin=(error.xyz+abs(q)*error.w)/(clip.w-error.w)+abs(q)*(4.0*1.1920928955078125e-7);
        lo=min(lo,vec4<f32>(q-margin,1.0));hi=max(hi,vec4<f32>(q+margin,1.0));
    }
    return mat2x4<f32>(lo,hi);
}
// renderlab: stage
