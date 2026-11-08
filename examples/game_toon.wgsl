// The game scene's toon program, a separate shading program from the photoreal props'
// (examples/game_scene.wgsl): banded direct light with a tinted shadow side, a view rim
// and a GI-occluded sky fill. Each object's material names its program; both quality
// paths shade every program of a frame with its own pipeline (docs/m7-mp1.md). Its
// `scatter` is flat Lambert, so the hero still bounces its colour into the GI cache.
// It declares no `reflection_lobe`: toon surfaces trace no reflections.
struct Parameters {
    shadow_tint:vec3<f32>, bands:f32,
    rim_color:vec3<f32>, rim_power:f32,
    thresholds:vec2<f32>, fill:f32,
};
const DEFAULT_PARAMETERS:Parameters = Parameters(
    vec3<f32>(0.12,0.08,0.24),4.0,
    vec3<f32>(0.12,0.2,0.3),4.0,
    vec2<f32>(0.12,0.9),1.0
);
struct Surface { n:vec3<f32>, base:vec3<f32> };

fn surface(input:SurfaceInput) -> Surface {
    let base=clamp(input.material.base_color.rgb*input.color.rgb*sample_base_color(input).rgb,vec3<f32>(0.0),vec3<f32>(1.0));
    // Decals (DC1) edit the resolved inputs; material_normal already includes theirs.
    return Surface(material_normal(input),decal_base_color(input,vec4<f32>(base,1.0)).rgb);
}

fn scatter(s:Surface, input:SurfaceInput, wi:vec3<f32>, wo:vec3<f32>) -> vec3<f32> {
    return s.base*(max(dot(s.n,wi),0.0)/PI);
}

fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> {
    let levels=max(floor(clamp(params.bands,2.0,16.0))-1.0,1.0);
    let low=clamp(params.thresholds.x,0.0,0.999);
    let high=clamp(params.thresholds.y,low+0.001,1.0);
    var exposed=exposed_product(indirect_irradiance(input,s.n),s.base*(max(params.fill,0.0)/PI));
    for(var i=0u;i<light_count();i+=1u) {
        let light=light_sample(i,input);
        let incident=clamp((dot(s.n,light.direction)-low)/(high-low),0.0,1.0);
        let band=floor(incident*levels+0.5)/levels;
        let visibility=light_visibility(input,light);
        let tint=mix(clamp(params.shadow_tint,vec3<f32>(0.0),vec3<f32>(1.0)),
            vec3<f32>(1.0),band*visibility);
        exposed+=exposed_product(light.intensity,s.base*tint*(light.attenuation/PI));
    }
    var v=unit_normal(input.eye-input.world);
    if dot(v,v)<0.5 { v=s.n; }
    let rim=pow(1.0-clamp(dot(s.n,v),0.0,1.0),max(params.rim_power,0.001));
    return exposed+max(params.rim_color,vec3<f32>(0.0))*rim;
}
