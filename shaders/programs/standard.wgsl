// Standard: the photoreal glTF metallic-roughness program, and the engine default. Its
// roughness filtering is shared by direct and environment specular. Edit and reload
// it like any program. Custom programs opt in through the same helpers.
// U1 fails the static normal-map/F3 gate; enable only for explicit experimentation.
const SPECULAR_AA:bool=false;
struct Surface { n:vec3<f32>, base:vec3<f32>, metal:f32, rough:f32 };

fn surface(input:SurfaceInput) -> Surface {
    let material=input.material;
    let sampled=sample_base_color(input);
    let mr=sample_metallic_roughness(input);
    let n=material_normal(input);
    let base=clamp(material.base_color.rgb*input.color.rgb*sampled.rgb*TINT,vec3<f32>(0.0),vec3<f32>(1.0));
    let metal=clamp(material.metallic*mr.b,0.0,1.0);
    let rough=filtered_roughness(clamp(material.roughness*mr.g,0.0,1.0),
        select(0.0,material_normal_variance(input)+geometric_normal_variance(input),SPECULAR_AA));
    return Surface(n,base,metal,rough);
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
    var exposed=exposed_product(environment_constant(),base*(1.0-metal)+f0*ab.x+vec3<f32>(ab.y));
    if environment_enabled() {
        let irradiance=environment_irradiance(n);
        let reflected=reflect(-v,n);
        let radiance=environment_reflection(reflected,rough);
        exposed=min(environment_product(irradiance,base*((1.0-metal)/PI))+
            environment_product(radiance,f0*ab.x+vec3<f32>(ab.y)),vec3<f32>(65504.0));
    }
    for(var i=0u;i<light_count();i+=1u) {
        let light=light_sample(i,input);
        let attenuation=light.attenuation*light_visibility(input,light);
        exposed=min(exposed+exposed_product(light.intensity,brdf(n,v,light.direction,base,metal,rough)*attenuation),vec3<f32>(65504.0));
    }
    return exposed;
}
