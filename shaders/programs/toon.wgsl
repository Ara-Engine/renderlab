// A deliberately non-PBR style, authored entirely through the shading-program API.
// Defaults live beside the typed block; materials only supply overrides.
// Rim colour is exposed HDR (intentionally independent of scene exposure).
struct Parameters {
    shadow_tint:vec3<f32>, bands:f32,
    rim_color:vec3<f32>, rim_power:f32,
    thresholds:vec2<f32>, fill:f32, flat_shaded:u32,
};
const DEFAULT_PARAMETERS:Parameters = Parameters(
    vec3<f32>(0.12,0.08,0.24),4.0,
    vec3<f32>(0.12,0.2,0.3),4.0,
    vec2<f32>(0.12,0.9),0.25,0u
);
struct Surface { albedo:vec3<f32>, n:vec3<f32> };

fn surface(input:SurfaceInput) -> Surface {
    let sampled=sample_base_color(input);
    let albedo=clamp(input.material.base_color.rgb*input.color.rgb*sampled.rgb,
        vec3<f32>(0.0),vec3<f32>(1.0));
    var n=input.geometric;
    if params.flat_shaded==0u && dot(input.normal,input.normal)>1e-12 {
        n=unit_normal(input.normal);
    }
    if dot(n,n)<0.5 { n=vec3<f32>(0.0,0.0,1.0); }
    if input.material.double_sided!=0u && !input.source_front { n=-n; }
    return Surface(albedo,n);
}

fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> {
    let levels=max(floor(clamp(params.bands,2.0,16.0))-1.0,1.0);
    let low=clamp(params.thresholds.x,0.0,0.999);
    let high=clamp(params.thresholds.y,low+0.001,1.0);
    var exposed=expose(s.albedo*ambient(s.n)*(max(params.fill,0.0)/PI));
    for(var i=0u;i<light_count();i+=1u) {
        let light=light_sample(i,input);
        let incident=clamp((dot(s.n,light.direction)-low)/(high-low),0.0,1.0);
        let band=floor(incident*levels+0.5)/levels;
        // Quantise the local lighting, then apply the engine's world-space shadow.
        // Keep soft visibility continuous so a penumbra is not another band boundary.
        let visibility=light_visibility(input,light);
        let tint=mix(clamp(params.shadow_tint,vec3<f32>(0.0),vec3<f32>(1.0)),
            vec3<f32>(1.0),band*visibility);
        exposed+=exposed_product(light.intensity,s.albedo*tint*(light.attenuation/PI));
    }
    var v=unit_normal(input.eye-input.world);
    if dot(v,v)<0.5 { v=s.n; }
    let rim=pow(1.0-clamp(dot(s.n,v),0.0,1.0),max(params.rim_power,0.001));
    return exposed+max(params.rim_color,vec3<f32>(0.0))*rim;
}
