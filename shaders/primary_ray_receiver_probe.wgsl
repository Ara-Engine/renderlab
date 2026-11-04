// Diagnostic receiver: same world/clip arithmetic, interpolation, strict Greater,
// draw order and indexed geometry as fs_receiver, plus the winning triangle ID.
enable primitive_index;
struct Camera { view_projection:mat4x4<f32>, view_mode:vec4<u32> };
@group(0) @binding(0) var<uniform> camera:Camera;
@group(1) @binding(0) var<storage,read> models:array<mat4x4<f32>>;
struct InstanceBase {value:u32,pad:vec3<u32>};
@group(2) @binding(0) var<uniform> instance_base:InstanceBase;
struct Output {
    @builtin(position) position:vec4<f32>,
    @location(0) world:vec3<f32>,
    @location(1) @interpolate(flat) instance:u32,
};
@vertex fn vs(@location(0) position:vec3<f32>,@location(4) slot:u32)->Output {
    let world=models[slot]*vec4<f32>(position,1.0);
    return Output(camera.view_projection*world,world.xyz,instance_base.value+slot+1u);
}
struct Receiver {
    @location(0) world:vec4<f32>,
    @location(1) id:vec4<u32>,
};
@fragment fn fs(input:Output,@builtin(primitive_index) primitive:u32)->Receiver {
    return Receiver(vec4<f32>(input.world,input.position.z),vec4<u32>(input.instance,primitive,0u,0u));
}
