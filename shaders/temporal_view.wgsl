// ---- MQ2 view dependence for TR1 (docs/m7-mq2.md), composed by src/program.rs ----
// Composed into the traced shading modules (both paths) of programs that declare
// `fn view_dependence(s:Surface, input:SurfaceInput) -> ViewDependence`. The shading entry
// stores the result in the HDR sample's alpha, which TR1 reads (temporal_resolve.wgsl) and
// then overwrites with 1.
//
// The hint is 1 + share / width: the program's view-dependent share of its output over the
// angle (radians) the reflected direction turns across one pixel: the camera's pixel angle,
// twice the shading normal's turn per pixel (curvature, as R1's ray cone) and twice the
// declared lobe's alpha. A view turn of theta radians then slides the view-dependent part by
// about theta * (hint - 1) pixels of its own image, share-weighted. 1 means view-independent.
fn tr1_view_hint(s:Surface,input:SurfaceInput,pixel:vec2<u32>,size:vec2<u32>)->f32 {
    let d=view_dependence(s,input);
    let share=clamp(d.share,0.0,1.0);
    if !(share>0.0) {return 1.0;}
    let camera=primary_ray(pixel,size);
    let pixel_angle=length(camera.dx_far-camera.dx_near)/max(length(camera.far-camera.near),1e-30);
    let curvature=2.0*max(length(input.normal_dx),length(input.normal_dy))/max(length(input.normal),1e-30);
    let width=pixel_angle+curvature+2.0*clamp(d.alpha,0.0,1.0);
    // Below binary16's largest finite value.
    return min(1.0+share/max(width,1e-4),60000.0);
}
// ---- end of MQ2 view dependence ----
