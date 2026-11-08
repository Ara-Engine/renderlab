// ---- generated raster lit entry, after the program ----
// Derivatives and the program's `surface` run in uniform control flow, before the
// engine's single-sided discard; discarded fragments output nothing, so survivors are
// unchanged by that order. Visibility uses the derivative plane, never the program's.
@fragment fn fs_lit(input:RasterInput, triangle:RasterTriangle, @builtin(front_facing) front:bool) -> @location(0) vec4<f32> {
    let source_front=front==(input.winding>0.0);
    // Shadow comparison keeps this geometric plane independent of the sampled shading normal.
    // Viewport Y increases downwards. Negate the derivative cross for the camera-facing normal,
    // then restore source orientation before the declared source-back frame reversal.
    let geometric=unit_normal(cross(unit_normal(dpdx(input.world)),unit_normal(dpdy(input.world))));
    let face=-geometric;
    let source_geometric=select(-face,face,source_front);
    let pixel=vec2<i32>(input.position.xy);
    let surface_input=decal_prepare(SurfaceInput(input.world,dpdx(input.world),dpdy(input.world),source_geometric,input.normal,dpdx(input.normal),dpdy(input.normal),input.tangent,input.handedness,
        input.uv,dpdx(input.uv),dpdy(input.uv),input.color,source_front,lighting.eye,vec2<u32>(pixel),material,
        raster_receiver(input.world,triangle,pixel,material.thin!=0u),decal_none()));
    let surface_output=surface(surface_input);
    if material.double_sided==0u && !source_front { discard; }
    rl_indirect_prepare(surface_input);
    return vec4<f32>(rl_finite(shade(surface_output,surface_input)),1.0);
}
