// wgpu attachment reader; the shared native resolve retains its buffer-only ABI.
// Hardware attachment hits are combined at the readers, avoiding a full-screen merge.
override HW_ATTACHMENT:bool=false;
override DEFER_REPAIR:bool=false;
@group(0) @binding(34) var hardware_ids:texture_2d<u32>;
@group(0) @binding(35) var hardware_depth:texture_depth_2d;
fn raster_visibility(pixel:vec2<u32>,width:u32)->vec2<u32> {
    let i=pixel.y*width+pixel.x;let sw=visibility[i];var v=sw;
    if HW_ATTACHMENT {
        let z=bitcast<u32>(textureLoad(hardware_depth,vec2<i32>(pixel),0));
        if z!=0u && z>=v.y {
            let payload=textureLoad(hardware_ids,vec2<i32>(pixel),0).x;
            if z>v.y || payload>v.x {v=vec2<u32>(payload,z);}
        }
    }
    if !DEFER_REPAIR && sw.y!=0u {visibility[i]=vec2<u32>(0u);}
    return v;
}
