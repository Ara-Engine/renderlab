// Exact per-pixel comparison. Read back only three mismatch counts on repeats.
@group(0) @binding(0) var current_hdr:texture_2d<f32>;
@group(0) @binding(1) var current_hit:texture_2d<u32>;
@group(0) @binding(2) var current_visibility:texture_2d<u32>;
@group(0) @binding(3) var current_tiles:texture_2d<u32>;
@group(0) @binding(4) var expected_hdr:texture_2d<f32>;
@group(0) @binding(5) var expected_hit:texture_2d<u32>;
@group(0) @binding(6) var expected_visibility:texture_2d<u32>;
@group(0) @binding(7) var expected_tiles:texture_2d<u32>;
@group(0) @binding(8) var<storage,read_write> counts:array<atomic<u32>,4>;
@compute @workgroup_size(8,8) fn compare(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=textureDimensions(current_hdr)) {return;}
    let p=vec2<i32>(id.xy);let tile=p/vec2<i32>(TRACE_X,TRACE_Y);
    let covered=textureLoad(current_tiles,tile,0).x!=0u;
    let expected_covered=textureLoad(expected_tiles,tile,0).x!=0u;
    if any(bitcast<vec4<u32>>(textureLoad(current_hdr,p,0))!=bitcast<vec4<u32>>(textureLoad(expected_hdr,p,0))) {
        atomicAdd(&counts[0],1u);
    }
    let hit=select(vec4<u32>(0u),textureLoad(current_hit,p,0),covered);
    let expected=select(vec4<u32>(0u),textureLoad(expected_hit,p,0),expected_covered);
    if any(hit!=expected) {atomicAdd(&counts[1],1u);}
    if atomicLoad(&counts[3])!=0u {
        let visible=select(256u,textureLoad(current_visibility,p,0).x,covered);
        let expected_visible=select(256u,textureLoad(expected_visibility,p,0).x,expected_covered);
        if visible!=expected_visible {atomicAdd(&counts[2],1u);}
    }
}
