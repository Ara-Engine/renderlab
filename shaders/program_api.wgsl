// ---- renderlab shading-program API, every stage ----
// A program defines `fn surface(input:SurfaceInput)->Surface` and
// `fn shade(s:Surface,input:SurfaceInput)->vec3<f32>` (exposed linear HDR) against these
// declarations, may declare `struct Parameters` (read as `params`), and declares no
// bindings. The engine composes prelude, this API, the stage API, the program and the
// generated entry point into one module per stage and visibility variant
// (src/program.rs). Programs see only their own receiver and footprint differentials,
// with no neighbouring lighting or screen buffers.

// Everything a program may read about its receiver. Raster frames fill it from
// interpolants and quad derivatives, traced frames from the exact pixel-centre hit.
struct SurfaceInput {
    // Receiver point in metres.
    world:vec3<f32>,
    // Pixel footprint on this triangle, without neighbouring lighting or visibility.
    world_dx:vec3<f32>,
    world_dy:vec3<f32>,
    // Unit plane normal of the receiver triangle on its source-front side.
    geometric:vec3<f32>,
    // Interpolated vertex normal and tangent, world space, not normalized.
    normal:vec3<f32>,
    // One-pixel derivatives of the interpolated vertex shading normal. Raster uses
    // quad derivatives; compute uses analytic differentials on this hit's triangle.
    // These contain no neighbouring lighting or visibility, and do not cross triangles.
    normal_dx:vec3<f32>,
    normal_dy:vec3<f32>,
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
    // DC1: authored world-space overlays, evaluated before surface(). Programs may
    // consume these directly or through material_base_color/metallic_roughness/normal.
    decals:DecalInput,
};

// One light's incidence before opaque visibility. TP2 filters its RGB through
// transparent sun casters. `direction` points toward the light; `index` is the
// light's position in the renderer's lighting vector.
struct LightSample { index:u32, direction:vec3<f32>, attenuation:f32, intensity:vec3<f32> };

// The lights that can reach this receiver (LC1, docs/m7-lc1.md): its camera cluster's
// lights in lighting-vector order, every directional and unlimited-range light included.
// Point and spot lights beyond their range contribute exactly zero, so the loop's sum is
// unchanged by the culling. Stages without a cluster walk every light. The count varies
// per receiver: keep implicit-derivative sampling outside the light loop.
fn light_count() -> u32 { return rl_light_total(); }

// The receiver's i-th light: the fixed light loop's direction and attenuation arithmetic.
fn light_sample(i:u32, input:SurfaceInput) -> LightSample {
    let index=rl_light_index(i);
    let light=lights[index];
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
    // TP2: coloured transparent shadows filter sun incidence on both opaque paths.
    let transmission=select(vec3<f32>(1.0),transparent_sun_transmission(input.world,l),light.kind.x==0u && light.kind.z!=0u);
    return LightSample(index,l,attenuation,light.rgb_inner.xyz*transmission);
}

// World-space visibility of the sampled light from this receiver, 1 when unshadowed:
// ray-traced on hardware renderers, light-space maps otherwise, never screen-space. The
// engine computes it at its own receiver, so it is identical for every program. Shadowed
// LC3 local lights use source-sampled rays or cached atlas maps. A conservative cluster
// may contain receivers outside a light's exact range/cone; those samples have no
// incidence and need no visibility work. This assumes no BRDF or receiver hemisphere.
fn light_visibility(input:SurfaceInput, s:LightSample) -> f32 {
    if s.attenuation==0.0 { return 1.0; }
    if lights[s.index].kind.w!=0u { return rl_local_shadow(lights[s.index],input.receiver,s.direction); }
    return hard_shadow(lights[s.index],input.receiver,s.direction);
}

// Unexposed environment irradiance (lux) around n: the prepared diffuse field, or pi
// times the constant environment.
fn ambient(n:vec3<f32>) -> vec3<f32> {
    if lighting.map.w!=0.0 {
        return environment_diffuse_at(environment_direction(n))*lighting.map.z;
    }
    return PI*lighting.environment;
}

// The prepared environment, or its constant radiance when no field is bound.
fn environment_enabled() -> bool { return lighting.map.w!=0.0; }
fn environment_constant() -> vec3<f32> { return lighting.environment; }
// Prepared diffuse irradiance and prefiltered radiance, before intensity and exposure;
// combine them with `environment_product`.
fn environment_irradiance(n:vec3<f32>) -> vec3<f32> {
    return environment_diffuse_at(environment_direction(n));
}
fn environment_reflection(direction:vec3<f32>, roughness:f32) -> vec3<f32> {
    return environment_radiance(environment_direction(direction),roughness);
}

// TP1 transparency (docs/m7-tp1.md): the material's last word packs a transparent
// material's opacity and IOR as halves; opaque materials store zero.
fn transparent_material(material:Material) -> bool { return material.pad!=0u; }
fn transparent_opacity(material:Material) -> f32 { return select(1.0,unpack2x16float(material.pad).x,material.pad!=0u); }
fn transparent_ior(material:Material) -> f32 { return select(1.5,unpack2x16float(material.pad).y,material.pad!=0u); }
// Coverage of a transparent material from its sampled alpha product; exactly 1 when opaque.
fn transparent_coverage(material:Material, alpha:f32) -> f32 {
    return select(1.0,clamp(transparent_opacity(material)*alpha,0.0,1.0),material.pad!=0u);
}

// Overflow-safe exposure of an unexposed radiance.
fn expose(radiance:vec3<f32>) -> vec3<f32> { return exposed_product(radiance,vec3<f32>(1.0)); }

// The HDR store contract: finite, nonnegative and at most 65504. NaN and negatives
// become 0; the identity on valid values.
fn rl_finite(c:vec3<f32>) -> vec3<f32> {
    return min(select(vec3<f32>(0.0),c,c>=vec3<f32>(0.0)),vec3<f32>(65504.0));
}
// A normal-map sample as three channels. A two-channel map (BC5, normal_mapped 2)
// stores X and Y; its Z is the unit normal's, rebuilt from the filtered sample. Every
// normal-map helper returns this form, so programs read all maps the same way.
fn rl_normal_texel(material:Material, value:vec4<f32>) -> vec4<f32> {
    if material.normal_mapped!=2u { return value; }
    let xy=value.xy*2.0-vec2<f32>(1.0);
    return vec4<f32>(value.xy,sqrt(max(1.0-dot(xy,xy),0.0))*0.5+0.5,value.a);
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
    if input.decals.normal_multiply==1.0 { return n; }
    let decaled=unit_normal(n*input.decals.normal_multiply+input.decals.normal_add);
    return select(n,decaled,dot(decaled,decaled)>0.5);
}

// Isotropic GGX lobe filtering. roughness is perceptual (alpha = roughness^2),
// variance is an additive squared-alpha spread. Zero is an exact identity.
// Custom programs may use their own lobe, scaling, anisotropy or no filtering.
fn filtered_roughness(roughness:f32, variance:f32) -> f32 {
    if variance<=0.0 { return roughness; }
    let alpha=roughness*roughness;
    return sqrt(sqrt(min(alpha*alpha+variance,1.0)));
}

// First-order normal-map spread. V3 alpha stores RMS Toksvig spread; legacy maps
// have no stored variance. RGB length also captures bilinear/trilinear direction
// cancellation. A small UNORM quantization dead zone keeps constant maps unchanged.
// Scale^2 is a small-slope approximation; zero scale eliminates map variance.
fn material_normal_variance(input:SurfaceInput) -> f32 {
    if input.material.normal_variance==2u {
        let covariance=normal_statistics(input).covariance;
        return min(covariance.x+covariance.z,1.0);
    }
    let sample=sample_normal_map(input);
    let mean_length=clamp(length(sample.xyz*2.0-vec3<f32>(1.0)),1e-6,1.0);
    var spread=max(2.0*(1.0-mean_length)/mean_length-0.008,0.0);
    if input.material.normal_variance==1u { spread+=sample.a*sample.a; }
    let scale=min(abs(input.material.normal_scale),1e10);
    return select(0.0,min(spread*scale*scale,1.0),input.material.normal_mapped!=0u);
}

// Box-footprint geometric normal spread (Kaplanyan/Tokuyoshi-style NDF filtering).
// Differentiate the normalized interpolant analytically, on both raster and compute.
// Does not detect subpixel face discontinuities: multi-triangle moments need U1 S-AA2.
fn geometric_normal_variance(input:SurfaceInput) -> f32 {
    let length2=dot(input.normal,input.normal);
    if length2<=1e-12 { return 0.0; }
    let n=unit_normal(input.normal);
    let dx=input.normal_dx-n*dot(n,input.normal_dx);
    let dy=input.normal_dy-n*dot(n,input.normal_dy);
    return min((dot(dx,dx)+dot(dy,dy))/(12.0*length2),1.0);
}

// ---- end of the shared program API ----

// Layer access records the same footprint as ordinary normal sampling, including residency.
fn sample_normal_map_grad(input:SurfaceInput,uv:vec2<f32>,dx:vec2<f32>,dy:vec2<f32>)->vec4<f32> {
    if input.material.normal_variance==2u {
        let value=sample_normal_layer_grad(input,uv,dx,dy,1).yzw;
        return vec4<f32>(unit_normal(value)*0.5+vec3<f32>(0.5),0.0);
    }
    return rl_normal_texel(input.material,sample_normal_layer_grad(input,uv,dx,dy,0));
}
