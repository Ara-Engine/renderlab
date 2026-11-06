// (cluster << 7 | triangle, depth bits) per pixel; depth 0 is a miss. Resolve zeroes
// the hits it reads, so the next frame starts clear without a full-buffer fill: every
// rasterized pixel lies in the placements' screen rectangle that resolve reads.
@group(0) @binding(29) var<storage,read_write> visibility:array<vec2<u32>>;
// visibility_buffer.wgsl's Cluster: primitives is the first primitive ID, or with
// counts bit 16 an offset of per-triangle source primitive IDs in the cluster data.
struct VisibilityCluster {lo:vec3<f32>,vertices:u32,hi:vec3<f32>,triangles:u32,owner:u32,refined:u32,primitives:u32,counts:u32,placement:u32,pad0:u32,pad1:u32,pad2:u32};
@group(0) @binding(31) var<storage,read> visibility_clusters:array<VisibilityCluster>;
struct VisibilityGroup {sphere:vec4<f32>,error:f32,pad0:u32,pad1:u32,pad2:u32};
@group(0) @binding(33) var<storage,read> visibility_groups:array<VisibilityGroup>;
// Scale of a coarse triangle's simplification error by which its receiver is lifted
// toward the viewer; 0 keeps receivers on the drawn triangle.
override LOD_LIFT:f32=0.0;
// Its own group: wgpu forbids binding arrays beside uniform buffers.
@group(2) @binding(8) var<storage,read> visibility_data:binding_array<PrimaryIndices,PRIMARY_BATCHES>;
// error: world-space simplification error of a coarse (LOD) triangle, 0 for source.
struct RasterHit {ids:vec2<u32>,positions:array<vec3<f32>,3>,error:f32};
// (placement + 1, primitive) of the visible triangle, or zero, and its object-space
// positions. Cluster triangles keep the index buffer's corner order and exact position
// bits, so these equal primary_triangle_vertices without its scattered gathers.
// Primitives at or beyond the mesh's source count are its appended LOD triangles.
fn raster_hit(pixel:vec2<u32>,width:u32)->RasterHit {
    var hit:RasterHit;
    let i=pixel.y*width+pixel.x;
    let v=visibility[i];
    if v.y==0u {return hit;}
    visibility[i]=vec2<u32>(0u);
    let c=visibility_clusters[v.x>>7u];let local=v.x&127u;
    let batch=primary_placements[c.placement]&0x1fffffffu;
    let d=&visibility_data[batch].data;
    var primitive=c.primitives+local;
    if (c.counts&0x10000u)!=0u {primitive=(*d)[c.primitives+local];}
    hit.ids=vec2<u32>(c.placement+1u,primitive);
    if c.refined!=0xffffffffu {
        // The conservative affine scale of the LOD projection (visibility_buffer_cull.wgsl).
        let m=models[c.placement];
        let a=abs(m[0].xyz);let b=abs(m[1].xyz);let e=abs(m[2].xyz);
        let columns=max(a.x+a.y+a.z,max(b.x+b.y+b.z,e.x+e.y+e.z));
        let rows=max(a.x+b.x+e.x,max(a.y+b.y+e.y,a.z+b.z+e.z));
        hit.error=visibility_groups[c.refined].error*sqrt(columns*rows);
    }
    let t=(*d)[c.triangles+local];
    for(var k=0u;k<3u;k+=1u) {
        let o=c.vertices+3u*((t>>(8u*k))&255u);
        hit.positions[k]=vec3<f32>(bitcast<f32>((*d)[o]),bitcast<f32>((*d)[o+1u]),bitcast<f32>((*d)[o+2u]));
    }
    return hit;
}
@group(0) @binding(13) var primary_hit_output:texture_storage_2d<rgba32uint,write>;
@group(0) @binding(16) var primary_world_output:texture_storage_2d<rgba32float,write>;
@group(0) @binding(17) var primary_normal_output:texture_storage_2d<rgba32float,write>;
@group(0) @binding(20) var primary_tile_output:texture_storage_2d<r32uint,write>;
@group(0) @binding(23) var primary_epsilon_output:texture_storage_2d<r32float,write>;
var<workgroup> covered:atomic<u32>;
override WRITE_RECEIVERS:bool=true;
// Reintersect the pixel-centre ray with the selected source triangle. Rebase beside
// the facet before the barycentric solve; raster-interpolated world positions and
// quantized depth are never receiver origins. Preserve receiver_point's FMA order.
fn raster_trace(pixel:vec2<u32>,size:vec2<u32>,ids:vec2<u32>,v:array<vec3<f32>,3>)->PrimaryTrace {
    let slot=ids.x-1u;
    let m=models[slot];let linear=mat3x3<f32>(m[0].xyz,m[1].xyz,m[2].xyz);
    let a=world_position(v[0],slot).xyz;let ab=linear*(v[1]-v[0]);let ac=linear*(v[2]-v[0]);
    let ray=primary_ray(pixel,size);let delta=ray.far-ray.near;let distance=length(delta);let direction=delta/distance;
    let local=fma(direction,vec3<f32>(primary_edge_dot(a-ray.near,direction)),ray.near)-a;
    let h=cross(direction,ac);let determinant=primary_edge_dot(ab,h);
    let u=primary_edge_dot(local,h)/determinant;let q=cross(local,ab);
    let w=primary_edge_dot(direction,q)/determinant;
    let weights=vec3<f32>(1.0-u-w,u,w);
    let point=world_position(receiver_point(v[0],v[1],v[2],weights),slot).xyz;
    return primary_repair(PrimaryTrace(vec4<u32>(ids,bitcast<vec2<u32>>(weights.yz)),point,v),ray.near,direction);
}
@compute @workgroup_size(8,4) fn resolve(@builtin(global_invocation_id) id:vec3<u32>,@builtin(workgroup_id) group:vec3<u32>) {
    let size=textureDimensions(primary_hit_output);
    var trace:PrimaryTrace;
    var lift=0.0;
    // As traced primaries: pixels outside the placements' screen rectangle are misses.
    let scene=primary_frame.scene;
    if all(id.xy<size) && all(id.xy>=scene.xy) && all(id.xy<scene.zw) {
        let hit=raster_hit(id.xy,size.x);
        if hit.ids.x!=0u {trace=raster_trace(id.xy,size,hit.ids,hit.positions);lift=hit.error*LOD_LIFT;atomicOr(&covered,1u);}
    }
    workgroupBarrier();
    if all(id.xy==group.xy*PRIMARY_TILE) {textureStore(primary_tile_output,group.xy,vec4<u32>(atomicLoad(&covered)));}
    if atomicLoad(&covered)==0u || any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);textureStore(primary_hit_output,p,trace.record);
    if trace.record.x==0u {return;}
    var receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,p);
    if lift>0.0 {
        // A coarse surface can lie below the source casters; move the shadow receiver to
        // the viewed side of its error band. Shading keeps the hit record unchanged.
        let ray=primary_ray(id.xy,size);
        let toward=ray.near-receiver.world;
        receiver.world+=select(-receiver.geometric,receiver.geometric,dot(receiver.geometric,toward)>=0.0)*lift;
    }
    textureStore(primary_epsilon_output,p,vec4<f32>(receiver.epsilon));
    if WRITE_RECEIVERS {
        textureStore(primary_world_output,p,vec4<f32>(receiver.world,receiver.epsilon));
        textureStore(primary_normal_output,p,vec4<f32>(receiver.geometric,select(1.0,-1.0,receiver.thin)));
    }
}
