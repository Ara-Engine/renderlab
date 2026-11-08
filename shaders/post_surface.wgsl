// Appended to the ray-free primary head. Uses TR1's hit/geometry reprojection convention.
struct PostFrame { previous:mat4x4<f32>,current:mat4x4<f32>,inverse:mat4x4<f32>,view:mat4x4<f32>,options:vec4<f32> };
@group(0) @binding(40) var<uniform> post_frame:PostFrame;
@group(0) @binding(41) var post_surface:texture_storage_2d<rgba32float,write>;
struct PostVec4s { data:array<vec4<f32>> };
struct PostU32s { data:array<u32> };
@group(1) @binding(2) var<storage,read> post_vertices:binding_array<PostVec4s,PRIMARY_BATCHES>;
@group(1) @binding(3) var<storage,read> post_indices:binding_array<PostU32s,PRIMARY_BATCHES>;
fn post_position(batch:u32,index:u32)->vec3<f32> {
    return post_vertices[batch].data[post_indices[batch].data[index]*3u].xyz;
}
fn post_store(p:vec2<u32>,point:vec4<f32>,depth:f32) {
    let size=vec2<f32>(textureDimensions(post_surface));
    let previous=post_frame.previous*point;
    let projected=post_frame.current*point;
    let current=(projected.xy/projected.w*vec2<f32>(0.5,-0.5)+0.5)*size;
    var velocity=vec2<f32>(0.0);
    if previous.w>0.0 && post_frame.options.z>0.5 {
        velocity=(current-(previous.xy/previous.w*vec2<f32>(0.5,-0.5)+0.5)*size)*post_frame.options.x;
        let radius=length(velocity);
        velocity*=min(1.0,post_frame.options.y/max(radius,1e-5));
    }
    textureStore(post_surface,p,vec4<f32>(velocity,depth,1.0));
}
@compute @workgroup_size(8,8) fn post_primary_surface(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(post_surface);if any(id.xy>=size) {return;}
    // Reconstruction can output more pixels than the current hit/depth samples.
    let internal=textureDimensions(primary_hit);
    let sample_pixel=min(vec2<u32>((vec2<f32>(id.xy)+0.5)*vec2<f32>(internal)/vec2<f32>(size)),internal-1u);
    var record=vec4<u32>(0u);
    if textureLoad(primary_tiles,sample_pixel/PRIMARY_TILE,0).x!=0u {record=textureLoad(primary_hit,sample_pixel,0);}
    if record.x==0u {
        let ray=primary_ray(sample_pixel,internal);let direction=normalize(ray.far-ray.near);
        post_store(id.xy,vec4<f32>(direction,0.0),1e8);return;
    }
    let slot=record.x-1u;let batch=primary_placements[slot]&0x1fffffffu;let i=record.y*3u;
    let a=post_position(batch,i);let b=post_position(batch,i+1u);let c=post_position(batch,i+2u);
    let uv=bitcast<vec2<f32>>(record.zw);
    let point=world_position(receiver_point(a,b,c,vec3<f32>(1.0-uv.x-uv.y,uv)),slot);
    post_store(id.xy,point,max(0.001,-(post_frame.view*point).z));
}
