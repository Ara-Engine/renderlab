// Merge fixed-function hardware visibility with the software raster's depth/ID word.
@group(0) @binding(0) var<storage,read_write> visibility:array<vec2<u32>>;
@group(0) @binding(1) var ids:texture_2d<u32>;
@group(0) @binding(2) var depth:texture_depth_2d;
@compute @workgroup_size(8,8) fn merge(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(ids);
    if any(id.xy>=size) {return;}
    let p=vec2<i32>(id.xy);let z=bitcast<u32>(textureLoad(depth,p,0));
    if z==0u {return;}
    let payload=textureLoad(ids,p,0).x;
    let i=id.y*size.x+id.x;let previous=visibility[i];
    if z>previous.y || (z==previous.y && payload>previous.x) {visibility[i]=vec2<u32>(payload,z);}
}
