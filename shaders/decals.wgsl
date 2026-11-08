// DC1: world-space grid of projected boxes; one read-only binding on every path.
// Each list is stable-sorted by authored order, with no overlap truncation.
struct DcTable { data:array<vec4<u32>> };
@group(3) @binding(21) var<storage,read> dc_table:DcTable;
fn dc_word(i:u32)->u32 { return dc_table.data[i/4u][i%4u]; }
fn dc_float(i:u32)->f32 { return bitcast<f32>(dc_word(i)); }
fn dc_vec4(i:u32)->vec4<f32> { return bitcast<vec4<f32>>(dc_table.data[i/4u]); }

// Affine blends act on the material's resolved inputs, never on final lighting.
// A user surface may use these directly, use material_* helpers, or ignore them.
struct DecalInput {
    base_multiply:vec4<f32>, base_add:vec4<f32>,
    // x roughness, y metallic.
    mr_multiply:vec2<f32>, mr_add:vec2<f32>,
    normal_multiply:f32, normal_add:vec3<f32>,
};
fn decal_none()->DecalInput {
    return DecalInput(vec4<f32>(1.0),vec4<f32>(0.0),vec2<f32>(1.0),vec2<f32>(0.0),1.0,vec3<f32>(0.0));
}
fn dc_texel(offset:u32,size:vec2<u32>,p:vec2<i32>)->vec4<f32> {
    let q=vec2<u32>(clamp(p,vec2<i32>(0),vec2<i32>(size)-vec2<i32>(1)));
    return dc_vec4(offset+4u*(q.x+q.y*size.x));
}
fn dc_bilinear(offset:u32,size:vec2<u32>,uv:vec2<f32>)->vec4<f32> {
    let p=uv*vec2<f32>(size)-vec2<f32>(0.5);let q=vec2<i32>(floor(p));let t=fract(p);
    return mix(mix(dc_texel(offset,size,q),dc_texel(offset,size,q+vec2<i32>(1,0)),t.x),
        mix(dc_texel(offset,size,q+vec2<i32>(0,1)),dc_texel(offset,size,q+vec2<i32>(1,1)),t.x),t.y);
}
fn dc_sample(texture:u32,uv:vec2<f32>,dx:vec2<f32>,dy:vec2<f32>,fallback:vec4<f32>)->vec4<f32> {
    if texture==0u { return fallback; }
    var size=vec2<u32>(dc_word(texture),dc_word(texture+1u));
    let levels=dc_word(texture+2u);
    let footprint=max(length(dx*vec2<f32>(size)),length(dy*vec2<f32>(size)));
    let lod=clamp(log2(max(footprint,1.0)),0.0,f32(levels-1u));
    let level=u32(lod);var offset=texture+4u;
    for(var j=0u;j<level;j+=1u) { offset+=4u*size.x*size.y;size=(size+vec2<u32>(1u))/2u; }
    let a=dc_bilinear(offset,size,uv);
    if level+1u>=levels { return a; }
    let next=offset+4u*size.x*size.y;
    return mix(a,dc_bilinear(next,(size+vec2<u32>(1u))/2u,uv),fract(lod));
}
fn decal_inputs(input:SurfaceInput)->DecalInput {
    var out=decal_none();
    if dc_word(0u)==0u { return out; }
    let low=vec3<f32>(dc_float(4u),dc_float(5u),dc_float(6u));
    let scale=vec3<f32>(dc_float(8u),dc_float(9u),dc_float(10u));
    let dims=vec3<u32>(dc_word(12u),dc_word(13u),dc_word(14u));
    let cell=(input.world-low)*scale;
    if any(cell<vec3<f32>(0.0)) || any(cell>vec3<f32>(dims)) { return out; }
    let c=min(vec3<u32>(cell),dims-vec3<u32>(1u));
    let list=dc_word(1u)+2u*(c.x+dims.x*(c.y+dims.y*c.z));
    let first=dc_word(list);let count=dc_word(list+1u);
    let geometric=select(-input.geometric,input.geometric,input.source_front);
    for(var j=0u;j<count;j+=1u) {
        let r=dc_word(2u)+48u*dc_word(first+j);
        let inverse=mat4x4<f32>(dc_vec4(r),dc_vec4(r+4u),dc_vec4(r+8u),dc_vec4(r+12u));
        let local=(inverse*vec4<f32>(input.world,1.0)).xyz;
        if any(abs(local)>vec3<f32>(0.5)) { continue; }
        let axis=dc_vec4(r+24u).xyz;
        let angle=smoothstep(dc_float(r+36u),dc_float(r+37u),dot(geometric,axis));
        let edge=clamp((0.5-max(max(abs(local.x),abs(local.y)),abs(local.z)))/max(0.5*dc_float(r+35u),1e-8),0.0,1.0);
        let coverage=dc_float(r+34u)*edge*angle;
        if coverage<=0.0 { continue; }
        let uv=local.xy+vec2<f32>(0.5);
        let dx=(inverse*vec4<f32>(input.world_dx,0.0)).xy;
        let dy=(inverse*vec4<f32>(input.world_dy,0.0)).xy;
        let texture=dc_word(r+44u);
        var base=dc_sample(texture,uv,dx,dy,vec4<f32>(1.0));
        if texture!=0u { base=vec4<f32>(base.rgb/max(base.a,1e-8),base.a); }
        base*=dc_vec4(r+28u);
        let alpha=clamp(base.a*coverage,0.0,1.0);
        let channels=dc_vec4(r+40u);let weight=alpha*channels.x;
        let blend=dc_word(r+47u);
        if blend==0u {
            out.base_multiply*=1.0-weight;out.base_add=out.base_add*(1.0-weight)+vec4<f32>(base.rgb,1.0)*weight;
        } else if blend==1u {
            let factor=mix(vec4<f32>(1.0),vec4<f32>(base.rgb,1.0),weight);
            out.base_multiply*=factor;out.base_add*=factor;
        } else { out.base_add+=vec4<f32>(base.rgb,0.0)*weight; }
        let mr=dc_sample(dc_word(r+45u),uv,dx,dy,vec4<f32>(1.0));
        let value=vec2<f32>(dc_float(r+32u)*mr.g,dc_float(r+33u)*mr.b);
        let mw=alpha*channels.yz;
        out.mr_multiply*=vec2<f32>(1.0)-mw;out.mr_add=out.mr_add*(vec2<f32>(1.0)-mw)+value*mw;
        let nw=alpha*channels.w;
        if nw>0.0 {
            let map=dc_sample(dc_word(r+46u),uv,dx,dy,vec4<f32>(0.5,0.5,1.0,1.0)).xyz*2.0-vec3<f32>(1.0);
            let normal=unit_normal(dc_vec4(r+16u).xyz*map.x+dc_vec4(r+20u).xyz*map.y+axis*map.z);
            out.normal_multiply*=1.0-nw;out.normal_add=out.normal_add*(1.0-nw)+normal*nw;
        }
    }
    return out;
}
fn decal_prepare(input:SurfaceInput)->SurfaceInput {
    var result=input;result.decals=decal_inputs(input);return result;
}
// Standard resolved material inputs; custom programs retain control of their Surface.
fn decal_base_color(input:SurfaceInput,base:vec4<f32>)->vec4<f32> {
    return clamp(base*input.decals.base_multiply+input.decals.base_add,vec4<f32>(0.0),vec4<f32>(1.0));
}
fn decal_metallic_roughness(input:SurfaceInput,mr:vec2<f32>)->vec2<f32> {
    return clamp(mr*input.decals.mr_multiply+input.decals.mr_add,vec2<f32>(0.0),vec2<f32>(1.0));
}
fn material_base_color(input:SurfaceInput)->vec4<f32> {
    return decal_base_color(input,input.material.base_color*input.color*sample_base_color(input));
}
fn material_metallic_roughness(input:SurfaceInput)->vec2<f32> {
    let sampled=sample_metallic_roughness(input);
    return decal_metallic_roughness(input,vec2<f32>(input.material.roughness*sampled.g,input.material.metallic*sampled.b));
}
fn material_opacity(input:SurfaceInput)->f32 { return material_base_color(input).a; }
