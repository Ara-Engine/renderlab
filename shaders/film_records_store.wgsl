// ---- T2 world-anchored sample records: the engine's store in the motion shading ----
// Every program's motion shading module (film_records.rs compose_shading) stores each pixel
// through these instead of writing primary_hdr directly. The program runs once per pixel on
// the frame's fresh sample, as in Interactive; its output is averaged with the records
// film_validate kept, and becomes the pixel's fresh record. Nothing reads colour to weight.
// This frame's fresh planes (the motion trace's film_plane).
@group(0) @binding(32) var<storage,read> film_planes:array<vec4<u32>>;
// This frame's moved records and tile bits (film_validate).
@group(0) @binding(42) var film_moved_now:texture_2d_array<u32>;
// This frame's fresh records.
@group(0) @binding(37) var film_fresh_out:texture_storage_2d<rgba32uint,write>;

// The background a miss shows and a background record contributes.
fn film_clear_colour()->vec4<f32> {
    return primary_frame.clear;
}
fn film_record_tile(pixel:vec2<i32>)->bool {
    let first=vec2<i32>((vec2<u32>(pixel)/PRIMARY_TILE)*PRIMARY_TILE);
    return (textureLoad(film_moved_now,first,0,0).w&FILM_TILE_BIT)!=0u;
}
// The sample the motion trace chose: its gradients follow that camera ray.
fn film_motion_begin(id:vec2<u32>,width:u32) {
    film_sample=film_offset(film_plane_index(film_planes[id.x+id.y*width]),id,width);
}
// Stores `sample`, the program output (or background) of the pixel's fresh sample with hit
// placement + 1 (zero: background). Pixels of tiles without explicit records store it as is.
// Elsewhere the pixel stores the mean of the sample and its kept records, in slot order, and
// the sample becomes its fresh record. A pixel without kept records therefore stores its sample
// unchanged: Interactive's image when the trace chose the centre.
fn film_motion_store(pixel:vec2<i32>,sample:vec3<f32>,placement:u32) {
    if !film_record_tile(pixel) {
        textureStore(primary_hdr,pixel,vec4<f32>(sample,1.0));
        return;
    }
    let width=textureDimensions(primary_hit).x;
    let plane=film_planes[u32(pixel.x)+u32(pixel.y)*width];
    let position=film_quantize(film_offset(film_plane_index(plane),vec2<u32>(pixel),width));
    var fresh=film_record(sample,placement,bitcast<f32>(plane.x),position,film_plane_side(plane));
    // Test diagnostics only: the trace's receiver-band suspect mark (never set otherwise).
    fresh.w|=select(0u,FILM_BAND,(plane.w&1u)!=0u);
    textureStore(film_fresh_out,pixel,fresh);
    let background=film_clear_colour().rgb;
    var sum=sample;
    var n=1.0;
    for(var s=0u;s+1u<FILM_RECORDS;s+=1u) {
        let r=textureLoad(film_moved_now,pixel,s,0);
        if (r.w&FILM_VALID)==0u {break;}
        sum+=film_colour(r,background);
        n+=1.0;
    }
    textureStore(primary_hdr,pixel,vec4<f32>(sum/n,1.0));
}
// A pixel the entry clears: uncovered tiles and the clear blocks outside the shaded region.
fn film_motion_clear(pixel:vec2<i32>) {
    if film_record_tile(pixel) {
        film_motion_store(pixel,film_clear_colour().rgb,0u);
        return;
    }
    textureStore(primary_hdr,pixel,film_clear_colour());
}
// ---- end of the T2 store ----
