// ---- renderlab shading-program API, every stage ----
// A program defines `fn surface(input:SurfaceInput)->Surface` and
// `fn shade(s:Surface,input:SurfaceInput)->vec3<f32>` (exposed linear HDR) against these
// declarations, may declare `struct Parameters` (read as `params`), and declares no
// bindings. The engine composes prelude, this API, the stage API, the program and the
// generated entry point into one module per stage and visibility variant
// (src/program.rs). Programs see only their own receiver: no derivatives, neighbours or
// screen buffers.

// Everything a program may read about its receiver. Raster frames fill it from
// interpolants and quad derivatives, traced frames from the exact pixel-centre hit.
struct SurfaceInput {
    // Receiver point in metres.
    world:vec3<f32>,
    // Unit plane normal of the receiver triangle on its source-front side.
    geometric:vec3<f32>,
    // Interpolated vertex normal and tangent, world space, not normalized.
    normal:vec3<f32>,
    tangent:vec3<f32>,
    handedness:f32,
    uv:vec2<f32>,
    // One pixel's uv footprint (raster: quad derivatives; traced: ray differentials).
    uv_dx:vec2<f32>,
    uv_dy:vec2<f32>,
    color:vec4<f32>,
    // The camera sees the triangle's source-front side.
    source_front:bool,
    eye:vec3<f32>,
    // Screen pixel, for stylisation only.
    pixel:vec2<u32>,
    // Engine-owned material factors and `thin` shadow declaration (0 or 1).
    material:Material,
    // Engine-private: pass `input` to the services below, never read it.
    receiver:Receiver,
};

// One light's unshadowed incidence. `direction` points toward the light.
struct LightSample { index:u32, direction:vec3<f32>, attenuation:f32, intensity:vec3<f32> };

fn light_count() -> u32 { return lighting.count; }

// The fixed light loop's direction and attenuation arithmetic, unchanged.
fn light_sample(i:u32, input:SurfaceInput) -> LightSample {
    let light=lights[i];
    var l=-light.travel_outer.xyz;
    var attenuation=1.0;
    if light.kind.x!=0u {
        // Scale coordinates before subtraction, avoiding overflow for opposite finite inputs.
        let half_delta=light.position_range.xyz*0.5-input.world*0.5;
        let scale=max(max(abs(half_delta.x),abs(half_delta.y)),abs(half_delta.z));
        l=unit_normal(half_delta);
        let scaled=half_delta/max(scale,1e-30);
        let distance=scale*min(2.0*sqrt(dot(scaled,scaled)),1e37/max(scale,1.0));
        let inverse=1.0/max(distance,0.01);
        attenuation=inverse*inverse;
        if light.position_range.w>0.0 {
            let ratio=min(distance/light.position_range.w,1.0);
            let window=max(1.0-ratio*ratio*ratio*ratio,0.0);
            attenuation*=window*window;
        }
        if light.kind.x==2u {
            let cone=clamp((dot(-l,light.travel_outer.xyz)-light.travel_outer.w)/max(light.rgb_inner.w-light.travel_outer.w,1e-4),0.0,1.0);
            attenuation*=cone*cone;
        }
    }
    return LightSample(i,l,attenuation,light.rgb_inner.xyz);
}

// World-space visibility of the sampled light from this receiver, 1 when unshadowed:
// ray-traced on hardware renderers, light-space maps otherwise, never screen-space. The
// engine computes it at its own receiver, so it is identical for every program.
fn light_visibility(input:SurfaceInput, s:LightSample) -> f32 {
    return hard_shadow(lights[s.index],input.receiver,s.direction);
}

// Unexposed environment irradiance (lux) around n: the prepared diffuse field, or pi
// times the constant environment.
fn ambient(n:vec3<f32>) -> vec3<f32> {
    if lighting.map.w!=0.0 {
        return environment_bilinear(environment_diffuse,environment_direction(n),0,0)*lighting.map.z;
    }
    return PI*lighting.environment;
}

// The prepared environment, or its constant radiance when no field is bound.
fn environment_enabled() -> bool { return lighting.map.w!=0.0; }
fn environment_constant() -> vec3<f32> { return lighting.environment; }
// Prepared diffuse irradiance and prefiltered radiance, before intensity and exposure;
// combine them with `environment_product`.
fn environment_irradiance(n:vec3<f32>) -> vec3<f32> {
    return environment_bilinear(environment_diffuse,environment_direction(n),0,0);
}
fn environment_reflection(direction:vec3<f32>, roughness:f32) -> vec3<f32> {
    return environment_radiance(environment_direction(direction),roughness);
}

// Overflow-safe exposure of an unexposed radiance.
fn expose(radiance:vec3<f32>) -> vec3<f32> { return exposed_product(radiance,vec3<f32>(1.0)); }

// The HDR store contract: finite, nonnegative and at most 65504. NaN and negatives
// become 0; the identity on valid values.
fn rl_finite(c:vec3<f32>) -> vec3<f32> {
    return min(select(vec3<f32>(0.0),c,c>=vec3<f32>(0.0)),vec3<f32>(65504.0));
}
// The standard material normal, including tangent repair, finite normal scale,
// mirrored handedness and source-back reversal. Safe on meshes without tangents.
// Call uniformly in raster stages, like the mesh-uv sampling helpers.
fn material_normal(input:SurfaceInput) -> vec3<f32> {
    let material=input.material;
    let normal_sample=sample_normal_map(input).xyz*2.0-vec3<f32>(1.0);
    var n=unit_normal(input.normal);
    if dot(input.normal,input.normal)<=1e-12 || dot(n,n)<0.5 { n=input.geometric; }
    if dot(n,n)<0.5 { n=vec3<f32>(0.0,0.0,1.0); }
    if material.normal_mapped!=0u {
        let projected_t=input.tangent-n*dot(n,input.tangent);
        var t=unit_normal(projected_t);
        if dot(projected_t,projected_t)<=1e-12 || dot(t,t)<0.5 {
            let axis=select(vec3<f32>(1.0,0.0,0.0),vec3<f32>(0.0,1.0,0.0),abs(n.x)>0.9);
            t=unit_normal(axis-n*dot(n,axis));
        }
        let b=input.handedness*cross(n,t);
        // Divide before multiplying to support every finite scale without overflow.
        let scale=max(abs(material.normal_scale),1.0);
        let decoded=vec3<f32>(normal_sample.xy*(material.normal_scale/scale),normal_sample.z/scale);
        var mapped=unit_normal(decoded);
        if length(decoded)<=1e-6/scale || dot(mapped,mapped)<0.5 { mapped=vec3<f32>(0.0,0.0,1.0); }
        n=unit_normal(t*mapped.x+b*mapped.y+n*mapped.z);
    }
    // A source-back surface reverses the entire perturbed frame.
    if material.double_sided!=0u && !input.source_front { n=-n; }
    return n;
}

// ---- end of the shared program API ----
