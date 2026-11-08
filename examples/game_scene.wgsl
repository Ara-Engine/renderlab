// The game scene's photoreal shading program, as shaders/programs/standard.wgsl, for the
// props. The hero uses its own toon program (examples/game_toon.wgsl): each material names
// its program, and both quality paths shade every program of a frame with its own
// pipeline (MP1, docs/m7-mp1.md).
// Materials request traced mirror and glossy reflections (M7-R1) through
// `reflection_lobe` unless they set `reflect` to 0. `glow` is emitted radiance (cd/m²,
// unexposed), as for lantern bulbs: it adds to the shaded colour only. The bulbs' light
// reaches other surfaces through the scene's point and spot lights; feeding a small, very
// bright emitter to the GI cache through the `emission` hook as well would double its
// light and turn it into fireflies (each cache cell traces a few fixed directions).
struct Parameters { glow:vec3<f32>, reflect:f32 };
const DEFAULT_PARAMETERS:Parameters = Parameters(vec3<f32>(0.0),1.0);
struct Surface { n:vec3<f32>, base:vec3<f32>, metal:f32, rough:f32 };

fn surface(input:SurfaceInput) -> Surface {
    let material=input.material;
    let sampled=sample_base_color(input);
    let mr=sample_metallic_roughness(input);
    let n=material_normal(input);
    let base=clamp(material.base_color.rgb*input.color.rgb*sampled.rgb*TINT,vec3<f32>(0.0),vec3<f32>(1.0));
    let metal=clamp(material.metallic*mr.b,0.0,1.0);
    let rough=clamp(material.roughness*mr.g,0.0,1.0);
    // Decals (DC1) edit the resolved inputs; material_normal already includes theirs.
    let decal_base=decal_base_color(input,vec4<f32>(base,1.0));
    let decal_mr=decal_metallic_roughness(input,vec2<f32>(rough,metal));
    return Surface(n,decal_base.rgb,decal_mr.y,decal_mr.x);
}

// The GGX lobe the engine traces on RT traced frames (roughness below 0.5); weight 0
// asks for no rays. Non-RT frames and rough surfaces keep the untraced estimate.
fn reflection_lobe(s:Surface, input:SurfaceInput) -> SpecularLobe {
    return SpecularLobe(s.n,s.rough*s.rough,params.reflect);
}

fn scatter(s:Surface, input:SurfaceInput, wi:vec3<f32>, wo:vec3<f32>) -> vec3<f32> {
    return brdf(s.n,wo,wi,s.base,s.metal,s.rough);
}

fn lobe_specular(n:vec3<f32>, v:vec3<f32>, lobe:EnvironmentLobe, f0:vec3<f32>, rough:f32) -> vec3<f32> {
    let l=lobe.direction;
    let nl=max(dot(n,l),0.0);
    if nl<=0.0 || dot(l+v,l+v)<1e-20 { return vec3<f32>(0.0); }
    let nv=max(dot(n,v),1e-7);
    let h=unit_normal(l+v);
    let nh=max(dot(n,h),0.0);
    let vh=clamp(dot(v,h),0.0,1.0);
    let alpha=max(rough*rough,1e-4);
    let a2=alpha*alpha;
    let wide=min(a2+0.5/(lobe.sharpness*max(vh,1e-4)),1.0);
    let c=cross(n,h);
    let denom=max(dot(c,c)+wide*nh*nh,1e-20);
    let d=wide/(PI*denom*denom);
    let visibility=0.5/max(nl*sqrt(a2+(1.0-a2)*nv*nv)+nv*sqrt(a2+(1.0-a2)*nl*nl),1e-20);
    return d*visibility*(f0+(vec3<f32>(1.0)-f0)*pow(1.0-vh,5.0))*nl;
}
fn lobe_weight(sharpness:f32, rough:f32) -> f32 {
    let alpha=max(rough*rough,1e-4);
    return clamp(0.4+0.12041*log2(max(sharpness*alpha*alpha,1e-30)),0.0,1.0);
}

fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> {
    let n=s.n;
    let base=s.base;
    let metal=s.metal;
    let rough=s.rough;
    var v=unit_normal(input.eye-input.world);
    if dot(v,v)<0.5 { v=n; }
    let ab=environment_dfg(dot(n,v),rough);
    let f0=vec3<f32>(0.04)*(1.0-metal)+base*metal;
    let diffuse=exposed_product(indirect_irradiance(input,n),base*((1.0-metal)/PI));
    let reflected=reflect(-v,n);
    let radiance=indirect_radiance(input,reflected,rough);
    let reflectance=f0*ab.x+vec3<f32>(ab.y);
    var specular=exposed_product(radiance,reflectance);
    if environment_enabled() {
        specular=environment_product(radiance,reflectance);
        for(var i=0u;i<environment_lobe_count();i+=1u) {
            let lobe=environment_lobe(input,i);
            let w=lobe_weight(lobe.sharpness,rough);
            if w<=0.0 { continue; }
            let share=environment_lobe_reflection(input,i,reflected,rough);
            specular+=w*(environment_product(lobe.illuminance,lobe_specular(n,v,lobe,f0,rough))
                -environment_product(share,reflectance));
        }
        specular=max(specular,vec3<f32>(0.0));
    }
    // Traced reflections replace the untraced sky specular by their traced fraction w,
    // which fades out across roughness 0.35-0.5 (and is 0 off the RT traced path).
    let traced=specular_incident(input);
    if traced.w>0.0 {
        specular=mix(specular,min(traced.rgb*reflectance,vec3<f32>(65504.0)),traced.w);
    }
    var exposed=min(diffuse+specular,vec3<f32>(65504.0));
    for(var i=0u;i<light_count();i+=1u) {
        let light=light_sample(i,input);
        let attenuation=light.attenuation*light_visibility(input,light);
        let response=brdf(n,v,light.direction,base,metal,rough);
        exposed=min(exposed+exposed_product(light.intensity,response*attenuation),vec3<f32>(65504.0));
    }
    return min(exposed+expose(max(params.glow,vec3<f32>(0.0))),vec3<f32>(65504.0));
}
