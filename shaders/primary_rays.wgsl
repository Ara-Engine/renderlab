// Traced primary visibility, shared by every primary module. Appended after the
// composed lit source, whose fs_lit body became primary_lit (primary_ray_gpu.rs).
// Draw geometry and materials are descriptor-indexed by batch (one per draw), so one
// dispatch covers every pixel. models/normals are packed by TLAS placement.
// sided.x: some draw is single-sided, so the trace culls candidate back faces.
struct PrimaryFrame { inverse:mat4x4<f32>, clear:vec4<f32>, sided:vec4<u32> };
// Hit record: placement+1 (zero is a miss), primitive, then f32 barycentrics.
@group(0) @binding(7) var primary_hit:texture_2d<u32>;
@group(0) @binding(10) var<uniform> primary_frame:PrimaryFrame;
@group(0) @binding(18) var primary_hdr:texture_storage_2d<rgba16float,write>;
struct PrimaryVec4s { data:array<vec4<f32>> };
struct PrimaryU32s { data:array<u32> };
struct PrimaryMaterial { value:Material };
@group(1) @binding(2) var<storage,read> primary_vertices:binding_array<PrimaryVec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> primary_indices:binding_array<PrimaryU32s,PRIMARY_BATCHES>;
@group(1) @binding(4) var<storage,read> primary_tangents:binding_array<PrimaryVec4s,PRIMARY_BATCHES>;
// Per placement: batch index; bit 31 tangent stream, bit 30 single-sided material.
@group(1) @binding(5) var<storage,read> primary_placements:array<u32>;
@group(2) @binding(10) var<storage,read> primary_materials:binding_array<PrimaryMaterial,PRIMARY_BATCHES>;
@group(2) @binding(11) var primary_base_color:binding_array<texture_2d<f32>,PRIMARY_BATCHES>;
@group(2) @binding(12) var primary_base_sampler:binding_array<sampler,PRIMARY_BATCHES>;
@group(2) @binding(13) var primary_metallic_roughness:binding_array<texture_2d<f32>,PRIMARY_BATCHES>;
@group(2) @binding(14) var primary_mr_sampler:binding_array<sampler,PRIMARY_BATCHES>;
@group(2) @binding(15) var primary_normal_map:binding_array<texture_2d<f32>,PRIMARY_BATCHES>;
@group(2) @binding(16) var primary_normal_sampler:binding_array<sampler,PRIMARY_BATCHES>;

// Near and far points of the pixel-centre ray and their per-pixel derivatives,
// unprojected through the same f32 matrix the raster path projects with.
struct PrimaryRay {near:vec3<f32>,far:vec3<f32>,dx_near:vec3<f32>,dy_near:vec3<f32>,dx_far:vec3<f32>,dy_far:vec3<f32>};
fn primary_ray(pixel:vec2<u32>,size:vec2<u32>)->PrimaryRay {
    let step=vec2<f32>(2.0,-2.0)/vec2<f32>(size);
    let ndc=(vec2<f32>(pixel)+vec2<f32>(0.5))/vec2<f32>(size)*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let inverse=primary_frame.inverse;
    // Reversed Z: near is 1, far is 0.
    let a=inverse*vec4<f32>(ndc,1.0,1.0);
    let b=inverse*vec4<f32>(ndc,0.0,1.0);
    let da=inverse[0]*step.x;let db=inverse[1]*step.y;
    let near=a.xyz/a.w;let far=b.xyz/b.w;
    return PrimaryRay(near,far,(da.xyz-near*da.w)/a.w,(db.xyz-near*db.w)/a.w,
        (da.xyz-far*da.w)/b.w,(db.xyz-far*db.w)/b.w);
}

// One covered pixel's triangle: batch, placement (the packed instance slot) and indices.
struct PrimaryHit {batch:u32,placement:u32,tangent:bool,ia:u32,ib:u32,ic:u32,bary:vec2<f32>};
fn primary_decode(record:vec4<u32>)->PrimaryHit {
    let placement=record.x-1u;let info=primary_placements[placement];let batch=info&0x3fffffffu;
    let i=record.y*3u;
    return PrimaryHit(batch,placement,(info>>31u)!=0u,primary_indices[batch].data[i],
        primary_indices[batch].data[i+1u],primary_indices[batch].data[i+2u],bitcast<vec2<f32>>(record.zw));
}
// World-space plane normal on the source-front side: the normal transform applied
// to the object-space winding normal, as the raster derivative normal measures it.
fn primary_geometric(a:vec3<f32>,b:vec3<f32>,c:vec3<f32>,slot:u32)->vec3<f32> {
    let n=normals[slot];
    let source=cross(unit_normal(b-a),unit_normal(c-a));
    return unit_normal(mat3x3<f32>(n.x.xyz,n.y.xyz,n.z.xyz)*unit_normal(source));
}
fn primary_input(batch:u32,index:u32,slot:u32)->VertexInput {
    let a=primary_vertices[batch].data[index*3u];let b=primary_vertices[batch].data[index*3u+1u];
    let c=primary_vertices[batch].data[index*3u+2u];
    return VertexInput(a.xyz,vec3<f32>(a.w,b.xy),b.zw,c,slot);
}
fn primary_vertex(hit:PrimaryHit,input:VertexInput,index:u32)->VertexOutput {
    if hit.tangent {return primary_tangent(input,primary_tangents[hit.batch].data[index]);}
    return vertex_output(input);
}
// Analytic ray differentials on the hit plane, mapped through the triangle's affine
// UV basis. A neighbouring pixel never supplies another triangle's derivative.
fn primary_gradients(ray:PrimaryRay,world:vec3<f32>,n:vec3<f32>,e1:vec3<f32>,e2:vec3<f32>,uv1:vec2<f32>,uv2:vec2<f32>)->mat2x2<f32> {
    let direction=ray.far-ray.near;let denominator=dot(n,direction);
    if abs(denominator)<1e-20 {return mat2x2<f32>();}
    let t=dot(n,world-ray.near)/denominator;
    let qx=ray.dx_near+t*(ray.dx_far-ray.dx_near);
    let qy=ray.dy_near+t*(ray.dy_far-ray.dy_near);
    let dx=qx-direction*(dot(n,qx)/denominator);let dy=qy-direction*(dot(n,qy)/denominator);
    // Scale before the dual basis so microtriangle edges stay normal floats.
    let scale=max(max(max(abs(e1.x),abs(e1.y)),abs(e1.z)),max(max(abs(e2.x),abs(e2.y)),abs(e2.z)));
    let a=e1/max(scale,1e-30);let b=e2/max(scale,1e-30);
    let area=dot(cross(a,b),n);
    if abs(area)<1e-20 {return mat2x2<f32>();}
    let u=cross(b,n)/area;let v=cross(n,a)/area;
    let sx=dx/max(scale,1e-30);let sy=dy/max(scale,1e-30);
    return mat2x2<f32>(uv1*dot(u,sx)+uv2*dot(v,sx),uv1*dot(u,sy)+uv2*dot(v,sy));
}

// Deferred material shading of every pixel with the unchanged lit body. Misses take
// the lit clear colour. Visibility is the module's hard_shadow: inline ray or counts.
// Sixteen-pixel groups shade the gather-bound soft image about a fifth faster than
// 32; primary_ray_gpu.rs SHADE_TILE must match.
@compute @workgroup_size(4,4) fn primary_shade(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(primary_hit);
    if any(id.xy>=size) {return;}
    let pixel=vec2<i32>(id.xy);let record=textureLoad(primary_hit,pixel,0);
    if record.x==0u {textureStore(primary_hdr,pixel,primary_frame.clear);return;}
    let hit=primary_decode(record);
    let ta=primary_input(hit.batch,hit.ia,hit.placement);let tb=primary_input(hit.batch,hit.ib,hit.placement);
    let tc=primary_input(hit.batch,hit.ic,hit.placement);
    let a=primary_vertex(hit,ta,hit.ia);let b=primary_vertex(hit,tb,hit.ib);let c=primary_vertex(hit,tc,hit.ic);
    let w=vec3<f32>(1.0-hit.bary.x-hit.bary.y,hit.bary);
    var input:VertexOutput;
    input.world=world_position(ta.position*w.x+tb.position*w.y+tc.position*w.z,hit.placement).xyz;
    let clip=camera.view_projection*vec4<f32>(input.world,1.0);
    input.position=vec4<f32>(vec2<f32>(id.xy)+vec2<f32>(0.5),clip.z/clip.w,1.0/clip.w);
    input.uv=a.uv*w.x+b.uv*w.y+c.uv*w.z;input.color=a.color*w.x+b.color*w.y+c.color*w.z;
    input.normal=a.normal*w.x+b.normal*w.y+c.normal*w.z;input.tangent=a.tangent*w.x+b.tangent*w.y+c.tangent*w.z;
    // Flat attributes come from the first vertex, as WebGPU's provoking vertex.
    input.winding=a.winding;input.handedness=a.handedness;
    let geometric=primary_geometric(ta.position,tb.position,tc.position,hit.placement);
    // The camera sees the source-front side: the raster front_facing of a source-front
    // view is that side compared with the instance winding.
    let source_front=dot(geometric,lighting.eye-input.world)>=0.0;
    let m=models[hit.placement];let linear=mat3x3<f32>(m[0].xyz,m[1].xyz,m[2].xyz);
    let e1=linear*(tb.position-ta.position);let e2=linear*(tc.position-ta.position);
    let gradients=primary_gradients(primary_ray(id.xy,size),input.world,geometric,e1,e2,tb.uv-ta.uv,tc.uv-ta.uv);
    // Raster's derivative normal points away from the camera.
    textureStore(primary_hdr,pixel,primary_lit(input,source_front==(input.winding>0.0),select(geometric,-geometric,source_front),gradients[0],gradients[1],hit.batch));
}
