// ---- View-mode store (src/program.rs view_variant) ----
// Appended to a traced or list shading module after `RL_VIEW_MODE`, `rl_view_data` and
// `rl_view_ambient`; the entry's lit store calls rl_view_store instead of `shade`. Modes are
// ViewMode's discriminants (src/material.rs).

// Data modes show misses as zero; LightingOnly shows the lit miss (sky, background).
fn primary_miss(id:vec2<u32>,size:vec2<u32>,clear:vec4<f32>)->vec4<f32> {
    if RL_VIEW_MODE==7u {return rl_lit_miss(id,size,clear);}
    return vec4<f32>(0.0,0.0,0.0,1.0);
}

// A white Lambert surface at normal n: the program's indirect irradiance (or the open
// environment's) and every light's direct light with the engine's visibility, exposed.
fn rl_view_lighting(n:vec3<f32>,input:SurfaceInput)->vec3<f32> {
    var exposed=exposed_product(rl_view_ambient(n,input),vec3<f32>(1.0/PI));
    for(var i=0u;i<light_count();i+=1u) {
        let light=light_sample(i,input);
        let cosine=dot(n,light.direction);
        if light.attenuation==0.0 || cosine<=0.0 {continue;}
        let attenuation=light.attenuation*light_visibility(input,light);
        exposed=min(exposed+exposed_product(light.intensity,vec3<f32>(cosine*attenuation/PI)),vec3<f32>(65504.0));
    }
    return min(exposed,vec3<f32>(65504.0));
}

fn rl_view_store(s:Surface,input:SurfaceInput)->vec4<f32> {
    let v=rl_view_data(s,input);
    switch RL_VIEW_MODE {
        case 0u: {return vec4<f32>(v.base_color,1.0);}
        case 1u: {return vec4<f32>(unit_normal(v.normal)*0.5+vec3<f32>(0.5),1.0);}
        case 2u: {return vec4<f32>(vec3<f32>(v.metallic),1.0);}
        case 3u: {return vec4<f32>(vec3<f32>(v.roughness),1.0);}
        case 5u: {return vec4<f32>(rl_finite(v.emission),1.0);}
        case 6u: {return vec4<f32>(vec3<f32>(v.occlusion),1.0);}
        // Alpha 1: view-independent for temporal reconstruction.
        case 7u: {return vec4<f32>(rl_view_lighting(unit_normal(v.normal),input),1.0);}
        default: {return vec4<f32>(0.0,0.0,0.0,1.0);}
    }
}
// ---- end of view-mode store ----
