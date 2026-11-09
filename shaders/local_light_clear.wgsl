// LC3 clears one cached atlas tile through its viewport/scissor, preserving neighbours.
@vertex fn vertex(@builtin(vertex_index) index:u32) -> @builtin(position) vec4<f32> {
    let p=vec2<f32>(f32((index<<1u)&2u),f32(index&2u));
    return vec4<f32>(p*2.0-vec2<f32>(1.0),0.0,1.0);
}
@fragment fn fragment() -> @builtin(frag_depth) f32 { return 0.0; }
