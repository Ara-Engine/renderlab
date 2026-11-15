// The wgpu HZB combines both rasters while reading, without an intermediate merge.
override HW_ATTACHMENT:bool=false;
@group(0) @binding(4) var hardware_depth:texture_depth_2d;
fn visibility_depth(pixel:vec2<u32>,width:u32)->f32 {
    var d=bitcast<f32>(visibility[pixel.y*width+pixel.x].y);
    if HW_ATTACHMENT {d=max(d,textureLoad(hardware_depth,vec2<i32>(pixel),0));}
    return d;
}
