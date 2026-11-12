// The luminance histogram of a view's displayed image (`Engine::view_histogram`): sRGB-encoded
// RGBA8 decoded to linear light, Rec. 709 luminance floored at 1e-4, binned by its log from
// `log_min` (bin 0) to 0 (white, the last bin). One invocation bins one sampled pixel: every
// `stride`-th pixel on each axis.
struct HistogramDesc { bins:u32, stride:u32, log_min:f32, pad:u32 };
@group(0) @binding(0) var image:texture_2d<f32>;
@group(0) @binding(1) var<storage,read_write> bins:array<atomic<u32>>;
@group(0) @binding(2) var<storage,read> desc:HistogramDesc;
fn decode(s:f32)->f32 {
    if s<=0.04045 { return s/12.92; }
    return pow((s+0.055)/1.055,2.4);
}
@compute @workgroup_size(8,8) fn view_histogram(@builtin(global_invocation_id) id:vec3<u32>) {
    let at=id.xy*desc.stride;
    let size=textureDimensions(image);
    if at.x>=size.x || at.y>=size.y { return; }
    let c=textureLoad(image,vec2<i32>(at),0).rgb;
    let l=max(0.2126*decode(c.r)+0.7152*decode(c.g)+0.0722*decode(c.b),1e-4);
    let x=(log(l)-desc.log_min)/(0.0-desc.log_min);
    atomicAdd(&bins[u32(clamp(floor(x*f32(desc.bins)),0.0,f32(desc.bins)-1.0))],1u);
}
