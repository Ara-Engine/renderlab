
// The target remains rgba32uint: low 16 bits are UNORM coverage, high 16 bits
// are the existing f16 weight. Bit 25 tags the cycle variant's count encoding.
const TR1_COVERAGE_COUNT:u32=0x2000000u;
fn tr1_unpack_coverage(layer:u32,word:u32)->vec2<f32> {
    let halves=unpack2x16float(word);
    if (layer&TR1_COVERAGE_COUNT)!=0u {
        return vec2<f32>(halves.x/max(halves.y,1.0),halves.y);
    }
    return vec2<f32>(unpack2x16unorm(word).x,halves.y);
}
fn tr1_pack_coverage(coverage:f32,weight:f32,counted:bool)->u32 {
    if counted {return pack2x16float(vec2<f32>(coverage*weight,weight));}
    return (pack2x16unorm(vec2<f32>(coverage,0.0))&0xffffu)
        |(pack2x16float(vec2<f32>(0.0,weight))&0xffff0000u);
}
