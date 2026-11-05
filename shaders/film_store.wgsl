// ---- T1 temporal film accumulation: the engine's store, after the program ----
// The temporal shading entry stores every pixel through film_store instead of writing
// primary_hdr directly (film_accumulation.rs FILM_ENTRY_SEAMS). History is the running mean of
// the program's exposed output and its sample weight, ping-ponged with the anchors, and kept
// only in trace tiles a camera ray has hit since the last reset.
@group(0) @binding(26) var film_colour_in:texture_2d<f32>;
@group(0) @binding(27) var film_colour_out:texture_storage_2d<rgba32float,write>;
@group(0) @binding(28) var film_anchor_out:texture_storage_2d<rg32uint,write>;
// This frame's tile flags (the trace's film_keep_tile output).
@group(0) @binding(30) var film_tiles_now:texture_2d<u32>;

// What a background sample contributes: the clear colour as the f16 image stores it. History
// that has seen only background therefore equals it exactly, as the reference integrates
// stored samples, and needs no storage.
fn film_background()->vec3<f32> {
    return quantizeToF16(primary_frame.clear.rgb);
}

// Static window: the cumulative mean becomes an exponential mean of weight 1/256 beyond it.
const FILM_WINDOW:f32=256.0;

// Accumulates one sample of the program's output at `pixel` (finite, nonnegative exposed
// HDR from rl_finite, or film_background) and stores the film value. Weights never read
// colour, so every program converges to the footprint integral of its own output.
// `placement` is the hit record's placement + 1 (zero: background); `distance` is the
// sample's view distance.
fn film_store(pixel:vec2<i32>,sample:vec3<f32>,placement:u32,distance:f32) {
    let tile=vec2<u32>(pixel)/PRIMARY_TILE;
    // No ray in this tile has hit since the reset: this sample and all before were background.
    if textureLoad(film_tiles_now,tile,0).x==0u {
        textureStore(primary_hdr,pixel,vec4<f32>(sample,1.0));
        return;
    }
    let k=film_index(pixel);
    var mean=sample;
    var n=1.0;
    if film_frame.reset==0u {
        // A tile hit for the first time starts from its implied background history.
        var history=vec4<f32>(film_background(),min(f32(film_frame.frames),FILM_WINDOW));
        if film_kept(pixel) {history=textureLoad(film_colour_in,pixel,0);}
        n=min(history.w+1.0,FILM_WINDOW);
        // m' = m + (x - m) / n', with the multiply-add pinned so repeats are bit-identical.
        mean=fma(sample-history.xyz,vec3<f32>(1.0/n),history.xyz);
    }
    textureStore(film_colour_out,pixel,vec4<f32>(mean,n));
    textureStore(film_anchor_out,pixel,vec4<u32>((placement&0x7fffffu)|(k<<24u),bitcast<u32>(distance),0u,0u));
    textureStore(primary_hdr,pixel,vec4<f32>(mean,1.0));
}
// ---- end of the T1 store ----
