// Post-processing pre-AA tonemap & colour-grade compute stage: evaluates HDR bloom,
// chromatic aberration, white balance, tonemapper (Reinhard / Filmic / AgX), contrast,
// saturation, 3D LUT and vignette, then writes perceptual sRGB + sRGB luma (alpha) into
// `graded_out` so post_tone.wgsl can run edge-directed FXAA 3.11 Quality + AMD FidelityFX CAS.
struct Settings {
    bloom_motion:vec4<f32>,
    lens:vec4<f32>,
    grade:vec4<f32>,
    white:vec4<f32>,
    display:vec4<f32>,
    extra:vec4<f32>, // bloom_threshold, anti_aliasing, sharpening, unused
};
@group(0) @binding(0) var colour:texture_2d<f32>;
@group(0) @binding(1) var bloom:texture_2d<f32>;
@group(0) @binding(2) var lut:texture_3d<f32>;
@group(0) @binding(3) var linear_sampler:sampler;
@group(0) @binding(4) var<uniform> settings:Settings;
@group(0) @binding(5) var graded_out:texture_storage_2d<rgba16float,write>;

fn hdr(uv:vec2<f32>)->vec3<f32> {
    let base=textureSampleLevel(colour,linear_sampler,uv,0.0).rgb;
    let b=textureSampleLevel(bloom,linear_sampler,uv,0.0).rgb;
    if settings.extra.x>0.0 {
        return base+b*settings.bloom_motion.x;
    }
    return mix(base,b,settings.bloom_motion.x);
}

fn agx_curve(v:vec3<f32>)->vec3<f32> {
    // Troy Sobotka's AgX default look (inset matrix -> log2 -> 6th-order sigmoid -> outset matrix).
    let inset=mat3x3<f32>(
        vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104),
    );
    let outset=mat3x3<f32>(
        vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116),
    );
    let c=inset*max(v,vec3<f32>(0.0));
    let log_c=clamp((log2(max(c,vec3<f32>(1e-10)))+12.47393)/16.5,vec3<f32>(0.0),vec3<f32>(1.0));
    let x2=log_c*log_c;
    let x4=x2*x2;
    let s=15.5*x4*x2-40.14*x4*log_c+31.96*x4-6.868*x2*log_c+0.4298*x2+0.1191*log_c-0.00232;
    let out_srgb=clamp(outset*s,vec3<f32>(0.0),vec3<f32>(1.0));
    return pow(out_srgb,vec3<f32>(2.2));
}

fn tone(x:vec3<f32>)->vec3<f32> {
    if settings.white.w>1.5 {
        return agx_curve(x);
    }
    if settings.white.w>0.5 {
        // Narkowicz's ACES-like fit, CC0 (2016); not a full ACES transform.
        // https://knarkowicz.wordpress.com/2016/01/06/aces-filmic-tone-mapping-curve/
        return clamp((x*(2.51*x+0.03))/(x*(2.43*x+0.59)+0.14),vec3<f32>(0.0),vec3<f32>(1.0));
    }
    return x/(1.0+x);
}

fn linear_to_srgb_ch(c:f32)->f32 {
    let x=clamp(c,0.0,1.0);
    return select(1.055*pow(x,1.0/2.4)-0.055,12.92*x,x<=0.0031308);
}
fn linear_to_srgb(c:vec3<f32>)->vec3<f32> {
    return vec3<f32>(linear_to_srgb_ch(c.r),linear_to_srgb_ch(c.g),linear_to_srgb_ch(c.b));
}

@compute @workgroup_size(8,8) fn grade(@builtin(global_invocation_id) id:vec3<u32>) {
    let size_u=textureDimensions(graded_out);
    if any(id.xy>=size_u) {return;}
    let size=vec2<f32>(size_u);
    let uv=(vec2<f32>(id.xy)+0.5)/size;
    let radial=uv*2.0-1.0;
    let shift=radial*dot(radial,radial)*settings.display.x/size;
    var x=hdr(uv);
    if settings.display.x>0.0 {x=vec3<f32>(hdr(uv+shift).r,x.g,hdr(uv-shift).b);}
    x=tone(max(x*settings.white.rgb,vec3<f32>(0.0)));
    x=0.18*pow(max(x/0.18,vec3<f32>(0.0)),vec3<f32>(settings.grade.x));
    let l=dot(x,vec3<f32>(0.2126,0.7152,0.0722));
    x=clamp(mix(vec3<f32>(l),x,settings.grade.y),vec3<f32>(0.0),vec3<f32>(1.0));
    if settings.display.y>0.0 {
        let n=settings.display.z;
        let coords=(x*(n-1.0)+0.5)/n;
        x=mix(x,textureSampleLevel(lut,linear_sampler,coords,0.0).rgb,settings.display.y);
    }
    let edge=smoothstep(0.15,1.8,dot(radial,radial));
    x=clamp(x*(1.0-settings.grade.z*edge),vec3<f32>(0.0),vec3<f32>(1.0));
    let srgb=linear_to_srgb(x);
    let luma=dot(srgb,vec3<f32>(0.299,0.587,0.114));
    textureStore(graded_out,vec2<i32>(id.xy),vec4<f32>(srgb,luma));
}
