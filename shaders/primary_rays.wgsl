// Traced primary visibility helpers, shared by the engine's trace kernels and every
// program's traced shading module (primary_ray_gpu.rs), after the composed prelude.
// models/normals are packed by TLAS placement.
// sided.x: some draw is single-sided, so the trace culls candidate back faces.
// scene: [x0, y0, x1, y1) pixels whose camera rays can reach a placement (scene_pixels).
// reflection (M7-R1/R1n, ReflectionSettings::frame_words): x the perceptual roughness where
// the traced reflection fraction reaches 0 (0: reflections off), y where it starts to fall.
// TR1's restart zones (temporal_reconstruction.rs `Planner::prepare`, tests in
// shaders/temporal_zones.wgsl). shadow_lo.w enables the moving caster's swept shadow volume
// (SK1); shadow_hi.w 0 rejects inside the box itself, 1 (one shadowed sun) inside the caster's
// swept bounds or where a ray toward the sun within its disk can meet them; shadow_light is the
// unit direction toward the sun (no zero component) and the tangent of its disk half-angle.
// relit: the restart spheres (xyz centre, w squared radius) of local lights whose intensity or
// colour changed this frame; counts.x of them.
struct Tr1Zones { shadow_lo:vec4<f32>, shadow_hi:vec4<f32>, shadow_light:vec4<f32>, relit:array<vec4<f32>,8>, counts:vec4<u32> };
struct PrimaryFrame { inverse:mat4x4<f32>, clear:vec4<f32>, sided:vec4<u32>, scene:vec4<u32>, sampling:vec4<u32>, reflection:vec4<f32> };
// Hit record: placement+1 (zero is a miss), primitive, then f32 barycentrics.
@group(0) @binding(7) var primary_hit:texture_2d<u32>;
@group(0) @binding(10) var<uniform> primary_frame:PrimaryFrame;
// Per placement: batch index; bit 31 tangent, bit 30 single-sided, bit 29 thin.
@group(1) @binding(5) var<storage,read> primary_placements:array<u32>;

fn primary_epsilon(v:array<vec3<f32>,3>,placement:u32)->f32 {
    let a=world_position(v[0],placement).xyz;let b=world_position(v[1],placement).xyz;
    let c=world_position(v[2],placement).xyz;
    return receiver_epsilon(a,b,c);
}
fn primary_receiver(trace_world:vec3<f32>,v:array<vec3<f32>,3>,placement:u32,pixel:vec2<i32>)->Receiver {
    return Receiver(trace_world,primary_geometric(v[0],v[1],v[2],placement),pixel,
        primary_placements[placement]&0x1fffffffu,primary_epsilon(v,placement),(primary_placements[placement]&0x20000000u)!=0u);
}

// Near and far points of the pixel-centre ray and their per-pixel derivatives,
// unprojected through the same f32 matrix the raster path projects with.
struct PrimaryRay {near:vec3<f32>,far:vec3<f32>,dx_near:vec3<f32>,dy_near:vec3<f32>,dx_far:vec3<f32>,dy_far:vec3<f32>};
fn primary_ray(pixel:vec2<u32>,size:vec2<u32>)->PrimaryRay {
    let step=vec2<f32>(2.0,-2.0)/vec2<f32>(size);
    let ndc=fma((vec2<f32>(pixel)+vec2<f32>(0.5))/vec2<f32>(size),vec2<f32>(2.0,-2.0),vec2<f32>(-1.0,1.0));
    let inverse=primary_frame.inverse;
    // Reversed Z: near is 1, far is 0.
    // Fix the contraction order: NVIDIA 617.14 changed an implicit matrix
    // multiply's arithmetic mid-run. This order preserves the accepted image.
    let xy=fma(inverse[0],vec4<f32>(ndc.x),inverse[1]*ndc.y);
    let a=xy+inverse[2]+inverse[3];
    let b=xy+inverse[3];
    let da=inverse[0]*step.x;let db=inverse[1]*step.y;
    let near=a.xyz/a.w;let far=b.xyz/b.w;
    return PrimaryRay(near,far,(da.xyz-near*da.w)/a.w,(db.xyz-near*db.w)/a.w,
        (da.xyz-far*da.w)/b.w,(db.xyz-far*db.w)/b.w);
}

// World-space plane normal on the source-front side: the normal transform applied
// to the object-space winding normal, as the raster derivative normal measures it.
fn primary_geometric(a:vec3<f32>,b:vec3<f32>,c:vec3<f32>,slot:u32)->vec3<f32> {
    let n=normals[slot];
    let source=cross(unit_normal(b-a),unit_normal(c-a));
    return unit_normal(mat3x3<f32>(n.x.xyz,n.y.xyz,n.z.xyz)*unit_normal(source));
}

// Pixels per trace workgroup and coverage flag; primary_ray_gpu.rs TRACE_TILE must match.
const PRIMARY_TILE=vec2<u32>(8u,4u);
// The trace's per-tile coverage flags, nonzero when a camera ray in the tile hit.
@group(0) @binding(21) var primary_tiles:texture_2d<u32>;
