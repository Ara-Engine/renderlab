// Standard: the photoreal glTF metallic-roughness program, and the engine default. Its
// direct-light BRDF optionally uses single-frame footprint integration (U1b). Custom
// programs opt in through the same helper. Sky specular is the occluded prefiltered
// radiance, with the environment's dominant lobes evaluated against this BRDF in place of
// their prefiltered share (M7-ES1, docs/m7-es1.md). Indirect light (sky and bounce) comes
// from the engine's inputs; `scatter` gives the world radiance cache this surface's own
// response, so the light it reflects reaches others as bounce light (M7-G2, docs/m7-g2.md).
// Its `reflection_lobe` asks the engine to trace mirror and glossy reflections on traced
// frames, which replace the untraced sky specular there (M7-R1, docs/m7-r1.md).
// Opt-in: the 256-sample filter passes quality, but misses the speed floor.
const SPECULAR_AA:bool=false;
// alpha: TP1 coverage of transparent materials (docs/m7-tp1.md), exactly 1 for opaque ones.
struct Surface { n:vec3<f32>, base:vec3<f32>, metal:f32, rough:f32, alpha:f32 };

fn surface(input:SurfaceInput) -> Surface {
    let material=input.material;
    let sampled=sample_base_color(input);
    let mr=sample_metallic_roughness(input);
    let n=material_normal(input);
    let base=clamp(material.base_color.rgb*input.color.rgb*sampled.rgb*TINT,vec3<f32>(0.0),vec3<f32>(1.0));
    let metal=clamp(material.metallic*mr.b,0.0,1.0);
    let rough=clamp(material.roughness*mr.g,0.0,1.0);
    // DC1: apply overlays to resolved inputs, before any BRDF or user shading.
    let decal_base=decal_base_color(input,vec4<f32>(base,1.0));
    let decal_mr=decal_metallic_roughness(input,vec2<f32>(rough,metal));
    let alpha=transparent_coverage(material,material.base_color.a*sampled.a*input.color.a);
    return Surface(n,decal_base.rgb,decal_mr.y,decal_mr.x,alpha);
}

// The data view modes show this program's resolved inputs, decals included.
fn view_data(s:Surface, input:SurfaceInput) -> ViewData {
    return ViewData(s.base,s.n,s.rough,s.metal,vec3<f32>(0.0),1.0);
}

// TP1: the share of the light behind a transparent surface that reaches the eye. A pane
// with an index of refraction above one is coloured glass: what its coverage and Fresnel
// reflectance leave, tinted by its base colour. At one (no interface: a plain alpha-blended
// film, as Ara declares a glTF BLEND material) it is what the coverage leaves, untinted and
// at every angle, as glTF, Unity and Unreal blend (`shade` blends the film's own light by
// coverage to match).
fn transmittance(s:Surface, input:SurfaceInput) -> vec3<f32> {
    let ior=transparent_ior(input.material);
    if ior<=1.0 { return vec3<f32>(1.0-s.alpha); }
    var v=unit_normal(input.eye-input.world);
    if dot(v,v)<0.5 { v=s.n; }
    let f0=((ior-1.0)/(ior+1.0))*((ior-1.0)/(ior+1.0));
    let fresnel=f0+(1.0-f0)*pow(1.0-clamp(abs(dot(s.n,v)),0.0,1.0),5.0);
    return (1.0-s.alpha)*(1.0-fresnel)*s.base;
}

// One environment lobe's GGX specular response, with exact Fresnel and visibility at its
// axis. The lobe's angular spread widens the NDF by 1/(2 sharpness v.h) in alpha^2: its
// variance mapped to half-vector slopes.
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
// How much of a lobe's prefiltered share its own evaluation replaces: all of it once the
// specular lobe is much wider than the light's (sharpness alpha^2 >= 31.6), none once it is
// much narrower (<= 0.1), where the N=V prefilter is the better estimate. Fitted on ES1's
// lobe grid.
fn lobe_weight(sharpness:f32, rough:f32) -> f32 {
    let alpha=max(rough*rough,1e-4);
    return clamp(0.4+0.12041*log2(max(sharpness*alpha*alpha,1e-30)),0.0,1.0);
}

// Traced reflections (M7-R1, docs/m7-r1.md): the engine traces this GGX lobe from each
// traced primary hit and shades every hit with its own program; `specular_incident` returns
// their mean exposed radiance, with the traced fraction in w. The weight is the luminance of
// the reflectance `shade` multiplies that radiance by, so the engine can skip pixels where
// what the trace could change stays invisible (stone's F0 of 0.04 against a chrome ball's 1).
// RL_DEFERRED: `shade` is linear in `specular_incident.rgb` at fixed w, mixing
// `traced x reflectance` in by the traced fraction, and reports both (reflection_report).
const RL_DEFERRED:bool=true;
fn reflection_lobe(s:Surface, input:SurfaceInput) -> SpecularLobe {
    var v=unit_normal(input.eye-input.world);
    if dot(v,v)<0.5 { v=s.n; }
    let film=transparent_material(input.material) && transparent_ior(input.material)<=1.0;
    let base=select(s.base*s.alpha,s.base,film);
    let ab=environment_dfg(dot(s.n,v),s.rough);
    let f0=vec3<f32>(0.04)*(1.0-s.metal)+base*s.metal;
    let reflectance=f0*ab.x+vec3<f32>(ab.y);
    return SpecularLobe(s.n,s.rough*s.rough,dot(reflectance,vec3<f32>(0.2126,0.7152,0.0722)));
}

// TR1's history hint (MQ2, docs/m7-mq2.md): the specular share of this surface's response
// under uniform light (its directional albedo against the diffuse albedo), which moves with
// the view like the GGX lobe above.
fn view_dependence(s:Surface, input:SurfaceInput) -> ViewDependence {
    var v=unit_normal(input.eye-input.world);
    if dot(v,v)<0.5 { v=s.n; }
    let ab=environment_dfg(dot(s.n,v),s.rough);
    let f0=vec3<f32>(0.04)*(1.0-s.metal)+s.base*s.metal;
    let specular=dot(f0*ab.x+vec3<f32>(ab.y),vec3<f32>(1.0));
    let diffuse=dot(s.base,vec3<f32>(1.0-s.metal));
    return ViewDependence(s.rough*s.rough,specular/max(specular+diffuse,1e-6));
}

// The radiance this surface sends toward wo per unit radiance arriving from wi: the same
// Lambert-plus-GGX BRDF its direct light uses, times the cosine at wi.
fn scatter(s:Surface, input:SurfaceInput, wi:vec3<f32>, wo:vec3<f32>) -> vec3<f32> {
    return brdf(s.n,wo,wi,s.base,s.metal,s.rough);
}

fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> {
    let n=s.n;
    // TP1: a transparent surface reflects diffusely over its coverage only. A plain blended
    // film (IOR one) blends all of its light by coverage instead (`coverage * colour`, glTF
    // BLEND), so a faint film keeps no full-strength specular sheen.
    let film=transparent_material(input.material) && transparent_ior(input.material)<=1.0;
    let base=select(s.base*s.alpha,s.base,film);
    let metal=s.metal;
    let rough=s.rough;
    var v=unit_normal(input.eye-input.world);
    if dot(v,v)<0.5 { v=n; }
    let ab=environment_dfg(dot(n,v),rough);
    let f0=vec3<f32>(0.04)*(1.0-metal)+base*metal;
    // Sky diffuse is the engine's incident irradiance at this receiver: occluded by the
    // world radiance cache when it runs, the open environment otherwise.
    let diffuse=exposed_product(indirect_irradiance(input,n),base*((1.0-metal)/PI));
    // Sky specular likewise: the occluded prefiltered radiance about the reflection, then
    // each dominant lobe the engine separates (only where it can occlude them) evaluated
    // against this BRDF in place of its share of that radiance.
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
    // Where the engine traced the lobe (mirror and glossy bands on traced frames), its
    // incident radiance replaces the untraced estimate, faded across the rough cutoff.
    reflection_report(reflectance,specular);
    let traced=specular_incident(input);
    if traced.w>0.0 {
        specular=mix(specular,min(traced.rgb*reflectance,vec3<f32>(65504.0)),traced.w);
    }
    var exposed=min(diffuse+specular,vec3<f32>(65504.0));
    for(var i=0u;i<light_count();i+=1u) {
        let light=light_sample_untinted(i,input);
        // Conservative clusters include zero-incidence candidates. The unfiltered
        // BRDF also returns exact zero below its shading hemisphere. Keep that
        // material-specific decision here, leaving user programs unrestricted.
        if light.attenuation==0.0 || (!SPECULAR_AA && dot(n,light.direction)<=0.0) { continue; }
        let visibility=light_visibility(input,light);
        // An unlit sample adds an exact zero (exposed_product keeps zeros), so it skips the
        // panes' transmission lookup and the BRDF.
        if visibility==0.0 { continue; }
        let attenuation=light.attenuation*visibility;
        var response=brdf(n,v,light.direction,base,metal,rough);
        if SPECULAR_AA {response=specular_brdf(input,light.direction,base,metal,rough);}
        if !rl_term(RL_TERM_BRDF) {response=vec3<f32>(1.0/PI);}
        let intensity=light.intensity*light_transmission(light,input);
        exposed=min(exposed+exposed_product(intensity,response*attenuation),vec3<f32>(65504.0));
    }
    // A plain blended film's light, blended by coverage (above).
    if film { exposed*=s.alpha; }
    return exposed;
}
