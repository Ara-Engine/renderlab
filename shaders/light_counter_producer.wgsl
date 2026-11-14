// Measurement-only per-pixel producer witness, joined to inline resolved material data.
struct LcPassCounter {header:vec4<u32>,bins:array<atomic<u32>>};
@group(0) @binding(80) var<storage,read_write> lc_counter:LcPassCounter;
@group(0) @binding(81) var<storage,read_write> lc_pass_records:array<vec4<u32>>;
var<private> lc_pass_current:bool;
var<private> lc_pass_calls:u32;
var<private> lc_pass_rays:u32;
var<private> lc_pass_pixel:u32;
fn lc_pass_add(field:u32,value:u32) {
    if lc_counter.header.x==0u {return;}
    let base=((3u*32u+lc_counter.header.y)*4u+3u)*96u;
    atomicAdd(&lc_counter.bins[base+field],value);
}
fn lc_pass_begin(index:u32) {
    lc_pass_current=lc_counter.header.x!=0u && index==lc_counter.header.y;
    lc_pass_calls=0u;lc_pass_rays=0u;
}
fn lc_pass_store(input:SurfaceInput,sample:LightSample,slot:u32) {
    if !lc_pass_current {return;}
    lc_pass_pixel=input.pixel.y*textureDimensions(primary_hit).x+input.pixel.x;
    lc_pass_records[lc_pass_pixel]=vec4<u32>(1u,lc_pass_calls,lc_pass_rays,slot|(u32(sample.attenuation==0.0)<<9u));
    lc_pass_add(89u,lc_pass_calls);
    lc_pass_add(95u,lc_pass_rays);
}
fn lc_pass_word() {
    lc_pass_add(94u,1u);
    if lc_pass_current {
        let record=lc_pass_records[lc_pass_pixel];
        lc_pass_records[lc_pass_pixel]=vec4<u32>(record.xyz,record.w|256u);
    }
}
