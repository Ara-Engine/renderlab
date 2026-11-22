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
@group(0) @binding(5) var graded:texture_2d<f32>;

@vertex fn vs(@builtin(vertex_index) id:u32)->@builtin(position) vec4<f32> {
    let p=vec2<f32>(f32((id<<1u)&2u),f32(id&2u));return vec4<f32>(p*2.0-1.0,0.0,1.0);
}
fn hdr(uv:vec2<f32>)->vec3<f32> {
    let base=textureSampleLevel(colour,linear_sampler,uv,0.0).rgb;
    let b=textureSampleLevel(bloom,linear_sampler,uv,0.0).rgb;
    if settings.extra.x>0.0 {
        return base+b*settings.bloom_motion.x;
    }
    return mix(base,b,settings.bloom_motion.x);
}
fn agx_curve(v:vec3<f32>)->vec3<f32> {
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
fn hash(p:vec2<u32>,frame:u32)->f32 {
    var h=p.x*0x9e3779b9u ^ p.y*0x85ebca6bu ^ frame*0xc2b2ae35u;
    h=(h^(h>>16u))*0x7feb352du;h=(h^(h>>15u))*0x846ca68bu;
    return f32((h^(h>>16u))>>8u)/16777216.0;
}
fn srgb_to_linear_ch(s:f32)->f32 {
    let x=clamp(s,0.0,1.0);
    return select(pow((x+0.055)/1.055,2.4),x/12.92,x<=0.04045);
}
fn srgb_to_linear(s:vec3<f32>)->vec3<f32> {
    return vec3<f32>(srgb_to_linear_ch(s.r),srgb_to_linear_ch(s.g),srgb_to_linear_ch(s.b));
}
fn load_graded(p:vec2<i32>,max_p:vec2<i32>)->vec4<f32> {
    return textureLoad(graded,clamp(p,vec2<i32>(0),max_p),0);
}
// Edge-directed FXAA 3.11 Quality in perceptual sRGB space (luma cached in alpha):
// smooths sub-pixel wires, window mullions, chains, twigs, merged-batch silhouettes and
// transparent glass edges while preserving surface micro-detail.
fn fxaa_quality(uv:vec2<f32>,ip:vec2<i32>,inv_size:vec2<f32>,max_p:vec2<i32>)->vec4<f32> {
    let c=load_graded(ip,max_p);
    let lM=c.a;
    let lN=load_graded(ip+vec2<i32>(0,-1),max_p).a;
    let lS=load_graded(ip+vec2<i32>(0,1),max_p).a;
    let lW=load_graded(ip+vec2<i32>(-1,0),max_p).a;
    let lE=load_graded(ip+vec2<i32>(1,0),max_p).a;
    let range_min=min(lM,min(min(lN,lS),min(lW,lE)));
    let range_max=max(lM,max(max(lN,lS),max(lW,lE)));
    let range=range_max-range_min;
    if range<max(0.0312,range_max*0.0625) {
        return vec4<f32>(c.rgb,0.0);
    }
    let lNW=load_graded(ip+vec2<i32>(-1,-1),max_p).a;
    let lNE=load_graded(ip+vec2<i32>(1,-1),max_p).a;
    let lSW=load_graded(ip+vec2<i32>(-1,1),max_p).a;
    let lSE=load_graded(ip+vec2<i32>(1,1),max_p).a;
    let lNS=lN+lS;
    let lWE=lW+lE;
    let subpix_rcp=1.0/range;
    let subpix_nswe=lNS+lWE;
    let edge_h1=-2.0*lW+lNW+lSW;
    let edge_v1=-2.0*lN+lNW+lNE;
    let edge_h2=-2.0*lM+lN+lS;
    let edge_v2=-2.0*lM+lW+lE;
    let edge_h3=-2.0*lE+lNE+lSE;
    let edge_v3=-2.0*lS+lSW+lSE;
    let edge_h=abs(edge_h1)+abs(edge_h2)*2.0+abs(edge_h3);
    let edge_v=abs(edge_v1)+abs(edge_v2)*2.0+abs(edge_v3);
    let is_horz=edge_h>=edge_v;
    let luma1=select(lW,lN,is_horz);
    let luma2=select(lE,lS,is_horz);
    let grad1=abs(luma1-lM);
    let grad2=abs(luma2-lM);
    let is_1_steepest=grad1>=grad2;
    let grad_scaled=0.25*max(grad1,grad2);
    var step_len=select(inv_size.x,inv_size.y,is_horz);
    var luma_local_avg=0.0;
    if is_1_steepest {
        step_len=-step_len;
        luma_local_avg=0.5*(luma1+lM);
    } else {
        luma_local_avg=0.5*(luma2+lM);
    }
    var cur_uv=uv;
    if is_horz {cur_uv.y+=step_len*0.5;} else {cur_uv.x+=step_len*0.5;}
    let offset=select(vec2<f32>(0.0,inv_size.y),vec2<f32>(inv_size.x,0.0),is_horz);
    var uv1=cur_uv-offset;
    var uv2=cur_uv+offset;
    var luma_end1=textureSampleLevel(graded,linear_sampler,uv1,0.0).a-luma_local_avg;
    var luma_end2=textureSampleLevel(graded,linear_sampler,uv2,0.0).a-luma_local_avg;
    var reached1=abs(luma_end1)>=grad_scaled;
    var reached2=abs(luma_end2)>=grad_scaled;
    let steps=array<f32,8>(1.0,1.5,2.0,2.0,2.0,2.0,4.0,8.0);
    for(var i=0u;i<8u;i+=1u) {
        if reached1 && reached2 {break;}
        let st=steps[i];
        if !reached1 {
            uv1-=offset*st;
            luma_end1=textureSampleLevel(graded,linear_sampler,uv1,0.0).a-luma_local_avg;
            reached1=abs(luma_end1)>=grad_scaled;
        }
        if !reached2 {
            uv2+=offset*st;
            luma_end2=textureSampleLevel(graded,linear_sampler,uv2,0.0).a-luma_local_avg;
            reached2=abs(luma_end2)>=grad_scaled;
        }
    }
    let dist1=select(uv.y-uv1.y,uv.x-uv1.x,is_horz);
    let dist2=select(uv2.y-uv.y,uv2.x-uv.x,is_horz);
    let is_dir1=dist1<dist2;
    let dist_final=min(dist1,dist2);
    let edge_thick=max(dist1+dist2,1e-6);
    let pixel_offset=-dist_final/edge_thick+0.5;
    let is_luma_center_smaller=lM<luma_local_avg;
    let correct_var=(select(luma_end2,luma_end1,is_dir1)<0.0)!=is_luma_center_smaller;
    var final_offset=select(0.0,pixel_offset,correct_var);
    // Subpixel aliasing blend for 1-pixel wires, twigs, and mullions.
    let luma_avg=(1.0/12.0)*(2.0*subpix_nswe+lNW+lNE+lSW+lSE);
    let subpix1=clamp(abs(luma_avg-lM)*subpix_rcp,0.0,1.0);
    let subpix2=(-2.0*subpix1+3.0)*subpix1*subpix1;
    let subpix_offset=subpix2*subpix2*0.75;
    final_offset=max(final_offset,subpix_offset);
    var final_uv=uv;
    if is_horz {final_uv.y+=final_offset*step_len;} else {final_uv.x+=final_offset*step_len;}
    return vec4<f32>(textureSampleLevel(graded,linear_sampler,final_uv,0.0).rgb,clamp(final_offset,0.0,1.0));
}
// Local contrast-adaptive sharpening (AMD FidelityFX CAS) in perceptual sRGB space, damped on
// pixels that FXAA just anti-aliased so micro-texture sharpens cleanly without re-aliasing edges.
fn cas_sharpen(base_srgb:vec3<f32>,ip:vec2<i32>,max_p:vec2<i32>,amount:f32)->vec3<f32> {
    if amount<=0.0 {return base_srgb;}
    let n=load_graded(ip+vec2<i32>(0,-1),max_p).rgb;
    let s=load_graded(ip+vec2<i32>(0,1),max_p).rgb;
    let w=load_graded(ip+vec2<i32>(-1,0),max_p).rgb;
    let e=load_graded(ip+vec2<i32>(1,0),max_p).rgb;
    let mn=min(base_srgb,min(min(n,s),min(w,e)));
    let mx=max(base_srgb,max(max(n,s),max(w,e)));
    let amp=sqrt(clamp(min(mn,vec3<f32>(1.0)-mx)/max(mx,vec3<f32>(1e-5)),vec3<f32>(0.0),vec3<f32>(1.0)));
    let peak=-1.0/mix(8.0,5.0,clamp(amount,0.0,1.0));
    let wt=amp*peak;
    return clamp((base_srgb+(n+s+w+e)*wt)/(vec3<f32>(1.0)+4.0*wt),vec3<f32>(0.0),vec3<f32>(1.0));
}
@fragment fn fs(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
    let aa_on=settings.extra.y>0.5;
    let sharp=settings.extra.z;
    if aa_on || sharp>0.0 {
        let size_u=textureDimensions(graded);
        let size=vec2<f32>(size_u);
        let uv=p.xy/size;
        let ip=vec2<i32>(p.xy);
        let max_p=vec2<i32>(size_u)-vec2<i32>(1);
        var srgb=load_graded(ip,max_p).rgb;
        var edge_blend=0.0;
        if aa_on {
            let aa=fxaa_quality(uv,ip,1.0/size,max_p);
            srgb=aa.rgb;
            edge_blend=aa.a;
        }
        if sharp>0.0 {
            let damped=sharp*(1.0-0.85*edge_blend);
            srgb=cas_sharpen(srgb,ip,max_p,damped);
        }
        // Subtle 1-LSB triangular-PDF dither in sRGB space eliminates 8-bit sky/fog banding.
        let r1=hash(vec2<u32>(p.xy),u32(settings.bloom_motion.w));
        let r2=hash(vec2<u32>(p.xy)+vec2<u32>(17u,31u),u32(settings.bloom_motion.w)^0x5a5au);
        srgb=clamp(srgb+vec3<f32>((r1+r2-1.0)/255.0),vec3<f32>(0.0),vec3<f32>(1.0));
        var x=srgb_to_linear(srgb);
        if settings.grade.w>0.0 {
            let signal=sqrt(clamp(dot(x,vec3<f32>(0.2126,0.7152,0.0722))*4.0,0.0,1.0));
            x+=(r1-0.5)*settings.grade.w*signal;
        }
        return vec4<f32>(clamp(x,vec3<f32>(0.0),vec3<f32>(1.0)),1.0);
    }
    let size=vec2<f32>(textureDimensions(colour));let uv=p.xy/size;
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
    x*=1.0-settings.grade.z*edge;
    if settings.grade.w>0.0 {
        let signal=sqrt(clamp(dot(x,vec3<f32>(0.2126,0.7152,0.0722))*4.0,0.0,1.0));
        x+=(hash(vec2<u32>(p.xy),u32(settings.bloom_motion.w))-0.5)*settings.grade.w*signal;
    }
    return vec4<f32>(clamp(x,vec3<f32>(0.0),vec3<f32>(1.0)),1.0);
}
