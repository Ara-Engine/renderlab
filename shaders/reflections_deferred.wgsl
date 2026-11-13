// ---- Deferred reflections, the program side (src/program.rs `deferred`) ----
// Composed into the traced shading modules of programs that declare `const RL_DEFERRED`
// while RENDERLAB_RL_DEFERRED is on. `shade` reports the reflectance it multiplies
// `specular_incident` by and the untraced specular it mixes it with; the entry writes both
// per pixel, multiplied by the pixel's fog and aerial transmittance (`atmosphere_surface` is
// `colour x t + s`, so a change to `shade` changes the stored colour by t times as much),
// with the traced fraction w. A composite after shading then adds
// `w x (reflectance x resolved - untraced)`: exactly what `shade` would have mixed in.
// The untraced output's alpha is the inline traced estimate's luminance, a diagnostic for
// tests (the composite ignores it).
@group(0) @binding(62) var rl_deferred_reflectance:texture_storage_2d<rgba16float,write>;
@group(0) @binding(63) var rl_deferred_untraced:texture_storage_2d<rgba16float,write>;
var<private> rl_reported:array<vec3<f32>,2>;
fn reflection_report(reflectance:vec3<f32>, untraced:vec3<f32>) {
    rl_reported[0]=reflectance;
    rl_reported[1]=untraced;
}
fn rl_deferred_clear(pixel:vec2<i32>) {
    textureStore(rl_deferred_reflectance,pixel,vec4<f32>(0.0));
    textureStore(rl_deferred_untraced,pixel,vec4<f32>(0.0));
}
fn rl_deferred_store(pixel:vec2<i32>, world:vec3<f32>) {
    let t=atmosphere_surface(vec3<f32>(1.0),world)-atmosphere_surface(vec3<f32>(0.0),world);
    textureStore(rl_deferred_reflectance,pixel,vec4<f32>(rl_reported[0]*t,rl_specular.w));
    textureStore(rl_deferred_untraced,pixel,vec4<f32>(rl_reported[1]*t,dot(rl_specular.rgb,RL_LUMA)));
}
// ---- end of deferred reflections ----
