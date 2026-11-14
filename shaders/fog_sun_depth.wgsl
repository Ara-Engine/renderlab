// Opaque, double-sided casters, matching the fog ray's FORCE_OPAQUE mask.
@group(0) @binding(0) var<uniform> fog_sun_clip:mat4x4<f32>;
@group(1) @binding(0) var<storage,read> fog_sun_models:array<mat4x4<f32>>;
@vertex fn fog_sun_vertex(@location(0) p:vec3<f32>,@builtin(instance_index) slot:u32)->@builtin(position) vec4<f32> {
    return fog_sun_clip*fog_sun_models[slot]*vec4<f32>(p,1.0);
}
