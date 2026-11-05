// ---- T1 temporal film accumulation: the engine's store, after the program ----
// The temporal shading entry stores every pixel through film_store instead of writing
// primary_hdr directly (program.rs Stage::Temporal). History is the running mean of the
// program's exposed output and its sample weight, ping-ponged with the anchors.
@group(0) @binding(26) var film_colour_in:texture_2d<f32>;
@group(0) @binding(27) var film_colour_out:texture_storage_2d<rgba32float,write>;
@group(0) @binding(28) var film_anchor_out:texture_storage_2d<rg32uint,write>;

// Static window: the cumulative mean becomes an exponential mean of weight 1/256 beyond it.
const FILM_WINDOW:f32=256.0;

// Accumulates one sample of the program's output at `pixel` (finite, nonnegative exposed
// HDR from rl_finite, or the clear colour) and stores the film value. Weights never read
// colour, so every program converges to the footprint integral of its own output.
// `placement` is the hit record's placement + 1 (zero: background); `distance` is the
// sample's view distance.
fn film_store(pixel:vec2<i32>,sample:vec3<f32>,placement:u32,distance:f32) {
    let k=film_index(pixel);
    var mean=sample;
    var n=1.0;
    if film_frame.reset==0u {
        let history=textureLoad(film_colour_in,pixel,0);
        n=min(history.w+1.0,FILM_WINDOW);
        // m' = m + (x - m) / n', with the multiply-add pinned so repeats are bit-identical.
        mean=fma(sample-history.xyz,vec3<f32>(1.0/n),history.xyz);
    }
    textureStore(film_colour_out,pixel,vec4<f32>(mean,n));
    textureStore(film_anchor_out,pixel,vec4<u32>((placement&0x7fffffu)|(k<<24u),bitcast<u32>(distance),0u,0u));
    textureStore(primary_hdr,pixel,vec4<f32>(mean,1.0));
}
// ---- end of the T1 store ----
