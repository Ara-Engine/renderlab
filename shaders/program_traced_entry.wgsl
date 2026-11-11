// ---- generated traced shading entry, after the program ----
// One covered pixel's triangle: batch, placement (the packed instance slot) and indices.
struct PrimaryHit {batch:u32,placement:u32,tangent:bool,ia:u32,ib:u32,ic:u32,bary:vec2<f32>};
fn primary_decode(record:vec4<u32>)->PrimaryHit {
    let placement=record.x-1u;let info=primary_placements[placement];let batch=info&0x1fffffffu;
    let i=record.y*3u;
    return PrimaryHit(batch,placement,(info>>31u)!=0u,primary_indices[batch].data[i],
        primary_indices[batch].data[i+1u],primary_indices[batch].data[i+2u],bitcast<vec2<f32>>(record.zw));
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

// primary_frame.scene widened to whole trace tiles, as [x0, y0, x1, y1): shading
// dispatches only its tiles, and every pixel outside it is a miss. Written by the CPU
// (primary_ray_gpu.rs PrimaryState::region) as storage, not derived from the uniform:
// a uniform-derived dispatch layout made long-lived pipelines change arithmetic mid-run.
@group(0) @binding(22) var<storage,read> primary_region:vec4<u32>;
// Pixels per clear block; primary_ray_gpu.rs CLEAR_TILE must match.
const PRIMARY_CLEAR_TILE=16u;
// Deferred shading of primary_region with this module's program. Misses take the lit
// clear colour (or the visible sky, primary_miss); uncovered tiles take it without reading
// their stale hit records.
// Visibility is the module's hard_shadow: inline ray or counts.
// One 32-invocation group shades an 8x4 region; primary_ray_gpu.rs SHADE_TILE must match.
// Mixed-program lists keep their independent 4x4 tiles.
// The dispatch holds the region's groups in its first rows, then one row per
// PRIMARY_CLEAR_TILE image rows of clear blocks: every pixel outside the region takes the
// clear colour from a few large blocks, because launching a shading group per
// background tile cost more than its stores (primary_ray_gpu.rs encode_primary_shade).
@compute @workgroup_size(8,4) fn primary_shade_tiles(@builtin(global_invocation_id) grid:vec3<u32>,@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let region=primary_region;
    let shaded=(region.zw-region.xy+vec2<u32>(7u,3u))/vec2<u32>(8u,4u);
    let size=textureDimensions(primary_hit);
    if group.y>=shaded.y {
        // Each lane stores one column of a clear block, skipping the region.
        if lane>=PRIMARY_CLEAR_TILE {return;}
        let base=vec2<u32>(group.x,group.y-shaded.y)*PRIMARY_CLEAR_TILE+vec2<u32>(lane,0u);
        for(var k=0u;k<PRIMARY_CLEAR_TILE;k+=1u) {
            let p=base+vec2<u32>(0u,k);
            if all(p<size) && !(all(p>=region.xy) && all(p<region.zw)) {textureStore(primary_hdr,vec2<i32>(p),primary_miss(p,size,primary_frame.clear));}
        }
        return;
    }
    if group.x>=shaded.x {return;}
    let id=grid+vec3<u32>(region.xy,0u);
    if any(id.xy>=size) || any(id.xy>=region.zw) {return;}
    if textureLoad(primary_tiles,id.xy/PRIMARY_TILE,0).x==0u {
        textureStore(primary_hdr,vec2<i32>(id.xy),primary_miss(id.xy,size,primary_frame.clear));
        return;
    }
    primary_shade_pixel(id.xy,size);
}
// A primary miss: `clear` (the lit clear colour), or with the visible sky on
// (primary_frame.sided.w, Renderer::set_visible_environment) the selected map along the
// pixel's camera ray at level 0, exposed, at the radiance scale whose f32 bits sided.w
// holds: the lighting's own intensity unless set_visible_environment_scale differs.
fn primary_miss(id:vec2<u32>,size:vec2<u32>,clear:vec4<f32>)->vec4<f32> {
    let ray=primary_ray(id,size);
    let d=normalize(ray.far-ray.near);
    // AT1: the atmosphere's sky (shaders/atmosphere_api.wgsl) unless the visible map is on.
    if primary_frame.sided.w==0u {return atmosphere_backdrop(d,clear);}
    let radiance=environment_radiance(environment_direction(d),0.0);
    let scale=bitcast<f32>(primary_frame.sided.w);
    if scale==lighting.map.z {return vec4<f32>(environment_product(radiance,vec3<f32>(1.0)),1.0);}
    return vec4<f32>(exposed_product(radiance,vec3<f32>(scale)),1.0);
}
// One pixel inside the image: its hit record shaded, or the clear colour for a miss.
// The program sees the same receiver frame as on raster, with explicit footprints.
fn primary_shade_pixel(id:vec2<u32>,size:vec2<u32>) {
    let pixel=vec2<i32>(id);let record=textureLoad(primary_hit,pixel,0);
    if record.x==0u {textureStore(primary_hdr,pixel,primary_miss(id,size,primary_frame.clear));return;}
    let hit=primary_decode(record);
    let ta=primary_input(hit.batch,hit.ia,hit.placement);let tb=primary_input(hit.batch,hit.ib,hit.placement);
    let tc=primary_input(hit.batch,hit.ic,hit.placement);
    let a=primary_vertex(hit,ta,hit.ia);let b=primary_vertex(hit,tb,hit.ib);let c=primary_vertex(hit,tc,hit.ic);
    let w=vec3<f32>(1.0-hit.bary.x-hit.bary.y,hit.bary);
    var input:VertexOutput;
    input.world=world_position(ta.position*w.x+tb.position*w.y+tc.position*w.z,hit.placement).xyz;
    // Keep the program's interpolation arithmetic while pinning the engine's
    // shadow point to R1's accepted barycentric contraction order.
    let receiver_world=world_position(receiver_point(ta.position,tb.position,tc.position,w),hit.placement).xyz;
    let clip=camera.view_projection*vec4<f32>(input.world,1.0);
    input.position=vec4<f32>(vec2<f32>(id)+vec2<f32>(0.5),clip.z/clip.w,1.0/clip.w);
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
    let gradients=primary_gradients(primary_ray(id,size),input.world,geometric,e1,e2,tb.uv-ta.uv,tc.uv-ta.uv);
    let bary_gradients=primary_gradients(primary_ray(id,size),input.world,geometric,e1,e2,vec2<f32>(1.0,0.0),vec2<f32>(0.0,1.0));
    let normal_dx=(b.normal-a.normal)*bary_gradients[0].x+(c.normal-a.normal)*bary_gradients[0].y;
    let normal_dy=(b.normal-a.normal)*bary_gradients[1].x+(c.normal-a.normal)*bary_gradients[1].y;
    // Raster's front_facing and derivative normal, which points away from the camera;
    // from here the receiver frame is built exactly as the raster entry builds it.
    let front=source_front==(input.winding>0.0);
    let receiver_geometric=select(geometric,-geometric,source_front);
    let lit_front=front==(input.winding>0.0);
    let face=-receiver_geometric;
    let source_geometric=select(-face,face,lit_front);
    let material=primary_materials[hit.batch].value;
    rl_load_parameters(hit.batch);
    let surface_input=decal_prepare(SurfaceInput(input.world,e1*bary_gradients[0].x+e2*bary_gradients[0].y,e1*bary_gradients[1].x+e2*bary_gradients[1].y,source_geometric,input.normal,normal_dx,normal_dy,input.tangent,input.handedness,
        input.uv,gradients[0],gradients[1],input.color,lit_front,lighting.eye,id,material,
        primary_receiver(receiver_world,array<vec3<f32>,3>(ta.position,tb.position,tc.position),hit.placement,pixel),decal_none()));
    // Engine inputs computed once per receiver (src/program.rs), before the program's
    // values are live.
    rl_indirect_prepare(surface_input);
    let surface_output=surface(surface_input);
    // LC1: after `surface` and any reflection hits, which walk every light.
    rl_lights_prepare(surface_input.world);
    // Unreachable: traced frames cull the source-back faces of single-sided draws.
    if material.double_sided==0u && !lit_front {textureStore(primary_hdr,pixel,vec4<f32>(0.0));return;}
    // AT1: the program's exposed output through the camera's fog and aerial perspective.
    // MQ2: alpha is the program's view dependence for TR1 (1 when undeclared).
    textureStore(primary_hdr,pixel,vec4<f32>(atmosphere_surface(rl_finite(shade(surface_output,surface_input)),input.world),tr1_view_hint(surface_output,surface_input,id,size)));
}
